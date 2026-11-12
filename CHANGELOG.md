# Changelog

## 0.8.0

### Agents (M3)

- A dead agent is visible immediately: `SHOW_CHILD_EXITED` marks the bound live
  agent `errored` (`reason: process_exit`) and emits `surface.exited`.
- Passive detection runs every second, off the main thread; `ps` no longer
  blocks the render path.
- Agent attention now reaches macOS Notification Center (previously only
  explicit `notification.create` did).
- `⌘⇧U` acknowledges the agent it visits; `agent.ack` clears derived attention
  and its unread notifications. Sidebar attention ranking now matches the core
  (error > needs_input).
- New `agent-ack` CLI and `make m3` acceptance script.
- The generated opencode plugin is now an OpenCode V2 `Plugin.define` with
  `id`/`setup` and an event-stream subscription; a CI test locks that shape so
  the "Expected object at [default]" load failure cannot return.

### Protocol and persistence (M2/M4)

- `session.save` is wired to the shell's real atomic snapshot + journal write.
- `session.restore` replaces live state after validation instead of appending.
- `berth schema` now emits a real JSON Schema from the Rust types at
  `docs/schema/berth.schema.json`; the CLI can run it without a daemon.
- Event contract fixed: `surface.exited`, `agent.attention_changed`,
  `workspace.layout`, `split.resize`, and group/move events are documented and
  advertised. The never-emitted `session.dirty` event was removed.
- `system.hello` reports the real transport auth mode; `system.ping` reports
  uptime.
- Journal is capped at 5 snapshots / 4 MiB; deterministic restore fuzzing and a
  replace-state test were added.

### Display and UI (M4)

- Font sizing is per-surface and per-display: 1× monitors get an automatic 2×
  point multiplier, overridable in Settings (`fontScale`, `0` = auto). Font
  changes apply live via `ghostty_surface_update_config`.
- Split weights round-trip: divider positions are restored and dragged
  positions are persisted with `split.resize`.
- Dedicated `⌘I` attention panel, one-screen onboarding, theme tokens, and a
  compact sidebar option.
- Startup, restore, save, core dispatch, socket auth/backpressure, and surface
  create/teardown now log under subsystem `dev.berth.app`; uncaught exceptions
  append to `~/Library/Logs/Berth/crash.log`.
- Configuration reference at `docs/config.md`.

### Known gaps

Headless app-test mode, peer-UID socket verification, hiding `BERTH_TOKEN` from
children, group management UI, geometric focus, and replayable journal events
remain in `docs/04-roadmap.md` (M5).
