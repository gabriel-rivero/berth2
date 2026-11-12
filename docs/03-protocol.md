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

Modes (config `socket` in `~/.config/berth/config.json`):

| Mode | Behaviour |
| --- | --- |
| `off` | no socket |
| `local` (default) | socket `0600`, same-uid only; mutating methods require the token |
| `trusted` | socket `0600`, same-uid only; no token (hooks convenience) |

Token: `~/Library/Application Support/Berth/token` (`0600`), also exported to children as `BERTH_TOKEN`. Clients pass it as a top-level `token` field (or inside `params`). Auth is uid + token, not process ancestry — ancestry checks break under tmux, `nohup`, editors, and remote shells, which is exactly where we need to work. Read-only methods (`*.list`, `*.get`, `system.*`, `events.subscribe`, `session.dump`) are allowed without a token in `local` mode; anything that reads terminal text or mutates state needs one. A failed check returns `permission_denied` with the request `id` and does not close the connection.

Limits: request lines above 8 MiB are rejected (`payload_too_large`) and the connection is closed, because the framing can no longer be trusted. If a client stops reading and its outbound queue exceeds 4 MiB, the server disconnects it rather than growing without bound.

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

- `seq` is process-wide and strictly monotonic. Each connection tracks its own cursor starting after the last event delivered (no duplicates on a single connection). On connect or reconnect the client passes `since=<seq>` via `events.subscribe` and receives replayed events from a bounded ring buffer (last 1024) or an explicit `{"event":"gap","data":{"dropped":N}}` before the surviving events.
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
| `workspace.list` | `{}` | `{workspaces:[Workspace], groups:[Group], selected_workspace, selected_surface}` |
| `workspace.layout` | `{workspace_id}` | `{workspace, panes:[Pane+active_surface], surfaces:[Surface], agents:[Agent]}` |
| `workspace.create` | `{name?, cwd?, group_id?, icon?, color?, focus?}` | `{workspace:Workspace}` |
| `workspace.get` | `{workspace_id}` | `{workspace:Workspace}` |
| `workspace.update` | `{workspace_id, name?, icon?, color?, group_id?, clear_group_id?, pinned?}` | `{workspace:Workspace}` |
| `workspace.select` | `{workspace_id}` | `{}` |
| `workspace.close` | `{workspace_id}` | `{}` |
| `workspace.move` | `{workspace_id, index}` | `{}` |

### group
| method | params | result |
| --- | --- | --- |
| `group.list` | `{}` | `{groups:[Group]}` |
| `group.create` | `{name, color?}` | `{group:Group}` |
| `group.update` | `{group_id, name?, color?, clear_color?, collapsed?}` | `{group:Group}` |
| `group.delete` | `{group_id}` | `{}` |

### pane
| method | params | result |
| --- | --- | --- |
| `pane.list` | `{workspace_id?}` | `{panes:[Pane]}` |
| `pane.focus` | `{pane_id}` | `{}` |
| `pane.resize` | `{pane_id, weight}` | `{}` |
| `split.resize` | `{split_id, index, weight}` | `{}` |
| `pane.close` | `{pane_id}` | `{}` |

### surface
| method | params | result |
| --- | --- | --- |
| `surface.list` | `{workspace_id?, pane_id?}` | `{surfaces:[Surface]}` |
| `surface.create` | `{pane_id?, cwd?, argv?, env?, focus?}` | `{surface_id}` |
| `surface.split` | `{surface_id, direction:"left"\|"right"\|"up"\|"down", cwd?, argv?, focus?}` | `{surface_id, pane_id}` |
| `surface.focus` | `{surface_id}` | `{surface_id}` |
| `surface.activate` | `{surface_id}` | `{surface_id}` |
| `surface.cycle` | `{surface_id, direction:"next"\|"prev"}` | `{surface_id}` |
| `surface.close` | `{surface_id}` | `{}` |
| `surface.update` | `{surface_id, title?, cwd?, progress?, clear_progress?, bell?, exited?}` | `{}` |
| `surface.send_text` | `{surface_id, text}` | `{}` |
| `surface.send_key` | `{surface_id, key:"enter"\|"ctrl-c"\|"esc"\|…, mods?:[…]}` | `{}` |
| `surface.read_text` | `{surface_id, scope:"screen"\|"scrollback", max_bytes?}` | `{text, truncated}` |
| `surface.resize` | `{surface_id, cols, rows}` | `{}` |

`surface.read_text` maps to `ghostty_surface_read_text`; `send_text`/`send_key` map to `ghostty_surface_text`/`ghostty_surface_key`. This is the automation surface: no accessibility tree, no screen scraping hacks.

### agent — the differentiator
| method | params | result |
| --- | --- | --- |
| `agent.register` | `{surface_id, provider, name?, agent_id?, resume?, meta?}` | `{agent:Agent}` |
| `agent.detect` | `{surface_id}` | `{agent:Agent\|null}` |
| `agent.update` | `{agent_id, state?, detail?, progress?, clear_progress?, session_ref?, meta?}` | `{agent:Agent}` |
| `agent.get` | `{agent_id}` | `{agent:Agent}` |
| `agent.list` | `{workspace_id?, state?}` | `{agents:[Agent]}` |
| `agent.clear` | `{agent_id}` | `{}` |
| `agent.ack` | `{agent_id}` | `{agent}` — clears derived attention and acks its notifications |
| `agent.resume` | `{agent_id}` | `{surface_id}` |

`agent.register` is idempotent per surface: re-registering rebinds rather than
duplicating. `agent_id` lets `berth run` mint the id before spawning so the
child can receive `BERTH_AGENT_ID`; registration then adopts that id. Omit it
and the core mints one.

`agent.detect` is the passive tier: it inspects the surface's foreground
process (`HostOps.pid_info`) and creates an `unknown` agent when the
command/argv matches a provider. It is idempotent and returns `null` when the
surface already has an agent or no provider is recognized. Passive agents have
no resume payload; only `berth run` or an explicit `agent.register` can supply
one.

Resume payloads are `{argv, cwd, session_ref?, env?}`. `env` carries the
launch environment (notably `PATH`) because Ghostty runs commands through a
login shell with a minimal environment; without it a resumed `claude` can miss
its interpreter. On restore, a provider session ref uses the provider template
(`claude --resume <ref>`, `codex resume <ref>`, `opencode --session <ref>`);
without a ref the saved argv is executed verbatim. A failed resume keeps the
surface in the tree and marks the agent `errored` with detail `resume failed`.
`agent.update` accepts `session_ref` to fill that ref once a hook observes the
provider's session id, without re-registering or losing the saved argv.
rather than silently starting a bare shell.

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
| `session.restore` | `{session:SessionSnapshot}` | `{restored:count}` |

### Events

| event | payload |
| --- | --- |
| `agent.created` / `agent.deleted` | `{agent}` / `{agent_id}` |
| `agent.state_changed` | `{agent_id, from, to, reason}` (`reason`: `update`, `stale`, `process_exit`) |
| `agent.attention_changed` | `{agent_id, attention, reason}` |
| `agent.progress` | `{agent_id, progress, detail?}` |
| `workspace.created` / `workspace.closed` / `workspace.changed` | `{workspace}` |
| `workspace.selected` | `{workspace_id}` |
| `workspace.moved` | `{workspace_id, index}` |
| `group.created` / `group.updated` | `{group}` |
| `group.deleted` | `{group_id}` |
| `pane.focus_changed` | `{pane_id, surface_id}` |
| `surface.created` / `surface.closed` | `{surface}` / `{surface_id}` |
| `surface.title_changed` | `{surface_id, title}` |
| `surface.pwd_changed` | `{surface_id, cwd}` |
| `surface.bell` | `{surface_id}` |
| `surface.progress` | `{surface_id, value}` (OSC 9;4) |
| `surface.exited` | `{surface_id}` — child exited; a bound live agent becomes `errored` |
| `notification.created` / `notification.acked` | `{notification}` / `{notification_id}` |

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
  "resume": {"argv": ["claude","--resume","8f1c…"], "cwd": "/Users/me/p/app", "session_ref": "8f1c…", "env": [["PATH","/opt/homebrew/bin:/usr/bin"]]},
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
5. `⌘⇧U` jumps to the newest unacked attention item; repeat to cycle through them.

No sound in v1. Per-provider mute is not implemented; macOS banners are
suppressed for the workspace currently being viewed.

## 7. Integration paths

Three tiers, all writing the same typed state — tier 1 needs no cooperation at all:

1. **Passive**: shell reports foreground pid/argv (`ghostty_surface_foreground_pid`, `tty_name`) plus Ghostty actions (title, OSC 9/99/777, progress, bell) → core matches provider table → `state: unknown`, live cwd/ports from `/proc`-equivalent (`lsof`), no registration.
2. **Wrapper**: `berth run -- claude` creates a surface with `BERTH_AGENT_ID`, registers on start, forwards exit status → `done`/`errored`. Zero per-project setup; the argv is recorded as the resume payload automatically.
3. **Hooks**: `berth hooks install` writes Claude Code `settings.json` hooks and an opencode plugin. Each event calls `berth hook claude` or `berth agent-set --surface …`, updating state/detail and recording the provider session id for `--resume` on the next launch. One socket round trip, so hook latency is negligible.

The wrapper is the recommended default: it makes Tier 2 (and therefore correct restore) universal.

## 8. CLI surface (thin, protocol-only)

```
berth ls [--json]                     # workspaces, groups, panes, surfaces, agents
berth split SURFACE_ID right|left|up|down
berth send SURFACE_ID "text"
berth send-key SURFACE_ID enter
berth read SURFACE_ID [--scrollback] [--max-bytes N]
berth notify "text" [--level needs_input] [--body B]
berth agent-register SURFACE_ID [--provider P] [--name N] [--id AGENT_ID]
berth agent-set AGENT_ID [--state S] [--detail D] [--progress 0.4] [--clear-progress] [--session-ref ID]
berth agent-set --surface SURFACE_ID [--state S] [--detail D]
berth hooks install [claude|opencode|all]  # idempotent; embeds this binary's path
berth hooks print [claude|opencode|all]    # recipes without writing
berth hook claude                     # stdin hook payload; used by generated config
berth run [--name N] [--provider P] [--cwd DIR] -- <argv…>  # creates surface, registers agent, execs
berth events [--types a,b] [--since N] # streams NDJSON
berth snapshot                        # session.dump to stdout
```

## 9. Compatibility rules

- `protocol` integer bumps only for breaking changes; additive methods/fields do not bump it.
- Deprecations: announce in `system.capabilities.features` for one minor release, then remove.
- JSON Schema for requests/events is generated from the Rust types and shipped in `docs/schema/` so non-Rust clients (Swift, TS, Python) can validate.
