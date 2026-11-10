# Berth — Protocol (v1 draft)

The protocol is the durable contract. Everything else (Swift shell, CLI, future Linux shell, editor plugins) is a client of it.

## 1. Transport

- Unix domain socket at `~/Library/Application Support/Berth/berth.sock` (override: `BERTH_SOCKET_PATH`).
- **NDJSON**: one JSON object per line, `\n`-terminated, UTF-8. Bidirectional: clients send requests, server sends responses **and** events on the same connection.
- No length prefixes, no binary framing. Payloads above 8 MiB are rejected (`payload_too_large`).

## 2. Handshake and auth

On connect the server sends a hello event; the client may then send `system.hello`.

```json
{"event":"hello","data":{"protocol":1,"app":"Berth","version":"0.1.0","instance":"inst_8c1d","auth":"local"}}
```

Modes (config `socket.mode`):

| Mode | Behaviour |
| --- | --- |
| `off` | no socket |
| `local` (default) | socket `0600`, same-uid only; mutating methods require the token |
| `trusted` | socket `0600`, same-uid only; no token (hooks convenience) |

Token: `~/Library/Application Support/Berth/token` (`0600`), also exported to children as `BERTH_TOKEN`. Auth is uid + token, not process ancestry — ancestry checks break under tmux, `nohup`, editors, and remote shells, which is exactly where we need to work. Read-only methods (`*.list`, `*.get`, `system.*`) are allowed without a token in `local` mode.

## 3. Envelope

Request:
```json
{"id":1,"method":"surface.split","params":{"surface_id":"srf_91c2","direction":"right"}}
```

Response:
```json
{"id":1,"ok":true,"result":{"surface_id":"srf_a7f0"}}
{"id":1,"ok":false,"error":{"code":"surface_not_found","message":"no surface srf_91c2","data":{}}}
```

Event:
```json
{"event":"agent.state_changed","seq":42,"ts":"2026-11-10T14:02:11.882Z","data":{"agent_id":"agt_44de","from":"thinking","to":"waiting_input","reason":"hook"}}
```

- `seq` is per-connection, strictly monotonic, starting after the last event already delivered (no gaps, no duplicates on a single connection). On reconnect the client passes `?since=<seq>` semantics via `events.subscribe {"since":41}` and receives replayed events from a bounded ring buffer (last 1024) or an explicit `{"event":"gap","data":{"dropped":N}}`.
- `ts` is RFC3339 UTC with milliseconds. **Model timestamps are epoch milliseconds** (`created_ms`, `since_ms`, `last_seen_ms`) — logs are for humans, state is for machines.

- Unknown methods → `method_not_found`; unknown params fields are ignored (forward-compatible), unknown *types* are errors.

## 4. Errors

| code | meaning |
| --- | --- |
| `bad_request` | malformed JSON / wrong types / missing param |
| `method_not_found` | unsupported method |
| `not_found` | id does not exist |
| `invalid_state` | legal method, illegal in current state |
| `permission_denied` | token missing/invalid |
| `payload_too_large` | > 8 MiB |
| `unsupported` | capability not compiled in (e.g. browser in v1) |
| `internal` | bug; `data.detail` carries context |

## 5. Methods (v1)

### system
| method | params | result |
| --- | --- | --- |
| `system.hello` | `{client_name?, client_version?}` | `{protocol, app, version, instance, auth}` |
| `system.ping` | `{}` | `{pong:true, uptime_ms}` |
| `system.capabilities` | `{}` | `{methods:[…], events:[…], features:[…]}` |

### workspace
| method | params | result |
| --- | --- | --- |
| `workspace.list` | `{}` | `{workspaces:[Workspace]}` |
| `workspace.create` | `{name?, cwd?, group_id?, icon?, color?}` | `{workspace:Workspace}` |
| `workspace.get` | `{workspace_id}` | `{workspace:Workspace}` |
| `workspace.update` | `{workspace_id, name?, icon?, color?, group_id?, pinned?}` | `{workspace:Workspace}` |
| `workspace.select` | `{workspace_id}` | `{}` |
| `workspace.close` | `{workspace_id}` | `{}` |
| `workspace.move` | `{workspace_id, index}` | `{}` |

### pane
| method | params | result |
| --- | --- | --- |
| `pane.list` | `{workspace_id}` | `{panes:[Pane]}` |
| `pane.focus` | `{pane_id}` | `{}` |
| `pane.resize` | `{pane_id, weight}` | `{}` |
| `pane.close` | `{pane_id}` | `{}` |

### surface
| method | params | result |
| --- | --- | --- |
| `surface.list` | `{workspace_id?, pane_id?}` | `{surfaces:[Surface]}` |
| `surface.create` | `{pane_id?, cwd?, argv?, env?, focus?}` | `{surface_id}` |
| `surface.split` | `{surface_id, direction:"left"\|"right"\|"up"\|"down", cwd?, argv?}` | `{surface_id, pane_id}` |
| `surface.focus` | `{surface_id}` | `{}` |
| `surface.close` | `{surface_id}` | `{}` |
| `surface.send_text` | `{surface_id, text}` | `{}` |
| `surface.send_key` | `{surface_id, key:"enter"\|"ctrl-c"\|"esc"\|…, mods?:[…]}` | `{}` |
| `surface.read_text` | `{surface_id, scope:"screen"\|"scrollback", max_bytes?}` | `{text, truncated}` |
| `surface.resize` | `{surface_id, cols, rows}` | `{}` |

`surface.read_text` maps to `ghostty_surface_read_text`; `send_text`/`send_key` map to `ghostty_surface_text`/`ghostty_surface_key`. This is the automation surface: no accessibility tree, no screen scraping hacks.

### agent — the differentiator
| method | params | result |
| --- | --- | --- |
| `agent.register` | `{surface_id, provider, name?, resume?, meta?}` | `{agent:Agent}` |
| `agent.update` | `{agent_id, state?, detail?, progress?, attention?, meta?}` | `{agent:Agent}` |
| `agent.get` | `{agent_id}` | `{agent:Agent}` |
| `agent.list` | `{workspace_id?, state?}` | `{agents:[Agent]}` |
| `agent.clear` | `{agent_id}` | `{}` |
| `agent.resume` | `{agent_id}` | `{surface_id}` |

`agent.register` is idempotent per surface: re-registering rebinds rather than duplicating.

### notification
| method | params | result |
| --- | --- | --- |
| `notification.create` | `{title, body?, workspace_id?, surface_id?, level?:"info"\|"needs_input"\|"error"}` | `{notification:Notification}` |
| `notification.list` | `{unread_only?}` | `{notifications:[Notification]}` |
| `notification.ack` | `{notification_id}` | `{}` |
| `notification.clear` | `{all?:true}` | `{}` |

### events
| method | params | result |
| --- | --- | --- |
| `events.subscribe` | `{types?:[…], since?:seq}` | `{subscribed:[…]}` |
| `events.unsubscribe` | `{types?:[…]}` | `{}` |

### session
| method | params | result |
| --- | --- | --- |
| `session.dump` | `{}` | `{session:SessionSnapshot}` |
| `session.save` | `{}` | `{path}` |
| `session.restore` | `{session:SessionSnapshot}` | `{}` |

### Events

| event | payload |
| --- | --- |
| `agent.created` / `agent.deleted` | `{agent}` / `{agent_id}` |
| `agent.state_changed` | `{agent_id, from, to, reason}` |
| `agent.progress` | `{agent_id, progress, detail?}` |
| `workspace.created` / `workspace.closed` / `workspace.changed` | `{workspace}` |
| `workspace.selected` | `{workspace_id}` |
| `pane.focus_changed` | `{pane_id, surface_id}` |
| `surface.created` / `surface.closed` | `{surface}` / `{surface_id}` |
| `surface.title_changed` | `{surface_id, title}` |
| `surface.pwd_changed` | `{surface_id, cwd}` |
| `surface.bell` | `{surface_id}` |
| `surface.progress` | `{surface_id, value}` (OSC 9;4) |
| `notification.created` / `notification.acked` | `{notification}` / `{notification_id}` |
| `session.dirty` | `{}` (debounced, for external sync tools) |

Events are derived from Ghostty actions where possible: `SET_TITLE`, `PWD`, `PROGRESS_REPORT`, `RING_BELL`, `COMMAND_FINISHED`, `DESKTOP_NOTIFICATION`, `OPEN_URL`, `SHOW_CHILD_EXITED`. That means OSC 9/99/777 notifications, OSC 7 cwd, and shell-integration command-finished work *without any Berth-specific hook*.

## 6. Agent schema

```jsonc
{
  "id": "agt_44de",
  "surface_id": "srf_91c2",
  "workspace_id": "ws_7f3a",
  "provider": "claude-code",          // claude-code | codex | opencode | custom
  "name": "fix-auth-refresh",         // human label; defaults to provider + short id
  "state": "waiting_input",           // see lifecycle
  "detail": "permission: allow edit src/auth.ts",  // one line, ≤120 chars
  "progress": 0.62,                   // 0..1 or null (unknown)
  "attention": "needs_input",         // none | info | needs_input | error
  "since_ms": 1762783331882,          // when state changed (epoch millis)
  "last_seen_ms": 1762783331882,      // last update or heartbeat
  "tokens": {"in": 41233, "out": 1904, "cost_usd": 0.41},
  "resume": {"argv": ["claude","--resume","8f1c…"], "cwd": "/Users/me/p/app", "session_ref": "8f1c…"},
  "meta": {}                           // provider-specific, ≤4 KiB
}
```

### Lifecycle

```
            register
               │
               ▼
            unknown ──update──► idle
                                 │ ▲
                    update       │ │ finished
                                 ▼ │
        ┌──────────► thinking ────┘
        │              │  ▲
        │   tool_call  │  │ tool_result
        │              ▼  │
        └──────────── tool
                       │
          needs input  ▼
                 waiting_input ──update/done──► done
                       │
                       └──► blocked        (external: permission, auth, crash prompt)
                              │
                              ▼
                           errored  ──► (terminal state; resume or clear)
```

- `unknown` is the honest default: a surface whose foreground process matches a known provider but which never registered. The sidebar shows a dim, unbadged provider glyph.
- Illegal transitions are rejected with `invalid_state`; `unknown | idle → *` is always allowed (recovery).
- **Staleness**: if `state ∈ {thinking, tool}` and no update/`last_seen` refresh for `agent.ttl` (default 180 s), core moves it to `blocked` with `reason:"stale"` and raises `attention: error`. Silent hangs are the common failure mode and must surface.
- `attention` is *derived* by core, not supplied: `waiting_input`→`needs_input`, `blocked/errored`→`error`, `done`→`info` (once), else `none`. Callers may only force it via `notification.create` level.

### Attention escalation (v1)

1. Pane ring (animated border, 1.2 s) on transition into `needs_input`/`error` for a non-focused pane.
2. Workspace unread badge in sidebar; sidebar row sorts attention-first, not by creation time.
3. `notification.created` event; in-app popover (`⌘I`) lists the ladder.
4. macOS notification, deduped per agent per 60 s, suppressed when the tab is focused.
5. `⌘⇧U` jumps to the newest unacked attention item; `⌘⌥U` cycles.

No sound in v1. Per-workspace and per-provider mute.

## 7. Integration paths

Three tiers, all writing the same typed state — tier 1 needs no cooperation at all:

1. **Passive**: shell reports foreground pid/argv (`ghostty_surface_foreground_pid`, `tty_name`) plus Ghostty actions (title, OSC 9/99/777, progress, bell) → core matches provider table → `state: unknown`, live cwd/ports from `/proc`-equivalent (`lsof`), no registration.
2. **Wrapper**: `berth run -- claude` creates a surface with `BERTH_AGENT_ID`, registers on start, forwards exit status → `done`/`errored`. Zero per-project setup; the argv is recorded as the resume payload automatically.
3. **Hooks**: `berth agent update --state waiting_input --detail "…"` called from Claude Code `settings.json` hooks / opencode plugin. Richer `detail`, `tool` state, progress. One socket round trip, so hook latency is negligible.

The wrapper is the recommended default: it makes Tier 2 (and therefore correct restore) universal.

## 8. CLI surface (thin, protocol-only)

```
berth ls [--json]                     # workspaces + agents + attention summary
berth agent new --provider … [--name …]
berth agent update --state … [--detail …] [--progress 0.4]
berth notify "text" [--level needs_input]
berth run -- <argv…>                  # wrapper: creates surface, registers agent, execs
berth events [--types a,b] [--json]   # streams NDJSON
berth send --surface srf_x "text"
berth snapshot                        # session.dump to stdout
```

## 9. Compatibility rules

- `protocol` integer bumps only for breaking changes; additive methods/fields do not bump it.
- Deprecations: announce in `system.capabilities.features` for one minor release, then remove.
- JSON Schema for requests/events is generated from the Rust types and shipped in `docs/schema/` so non-Rust clients (Swift, TS, Python) can validate.
