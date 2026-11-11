# Berth — Architecture

## 1. Shape of the system

```
┌──────────────────────────────────────────────────────────────┐
│ Berth.app (Swift 6, AppKit) — the only process in v1          │
│                                                              │
│  ┌────────────────┐   ┌──────────────────────────────────┐   │
│  │ GhosttyHost    │   │ WorkspaceUI                      │   │
│  │ (C adapter)    │   │ sidebar · split tree · tabs ·    │   │
│  │ ghostty_app_t  │   │ palette · notifications          │   │
│  │ ghostty_surface│   └──────────────────────────────────┘   │
│  └───────┬────────┘                │                         │
│          │ NSView + CAMetalLayer   │                         │
│          │                         ▼                         │
│          │              ┌───────────────────────┐            │
│          │              │ BerthCore (Rust,      │            │
│          │              │ static lib, UniFFI)   │            │
│          │              │ model · state machine │            │
│          │              │ notifications · codec │            │
│          │              │ persistence · adapters│            │
│          │              └───────────┬───────────┘            │
│          │                          │                        │
│          │              ┌───────────▼───────────┐            │
│          │              │ Socket server (Swift) │            │
│          │              │ NDJSON JSON-RPC       │            │
│          │              └───────────┬───────────┘            │
└──────────┼──────────────────────────┼────────────────────────┘
           │                          │ berth.sock
   PTY + child procs           ┌──────▼──────┐
   owned by Ghostty            │ berth CLI   │  + hooks/wrappers
                               └─────────────┘
```

Two hard constraints drive everything:

1. **`ghostty_surface_new` makes Ghostty own the PTY and the child process.** There is no external-PTY mode and no "feed me output" API. The renderer, parser, and PTY are one unit. A daemon that owns PTYs *cannot* be rendered by GhosttyKit.
2. **libghostty-vt 1.0 is emulation only — no renderer.** Ghostty-grade text (shaping, ligatures, glyph atlas, GPU blit, IME, link detection, selection) lives in the Ghostty *app*, not the library.

Therefore v1 is one process, and portability is bought at the domain/protocol layer instead of the process layer.

## 2. Layers

### 2.1 Engine — GhosttyKit (pinned)

- Ghostty as a git submodule at a **tag**, not `main`. Built with `zig build -Doptimize=ReleaseFast` into `GhosttyKit.xcframework`, linked statically.
- One `ghostty_app_t` for the process; one `ghostty_surface_t` per terminal; one `NSView` subclass per surface owning a `CAMetalLayer`.
- App loop: `ghostty_app_tick` on a CVDisplayLink, plus immediate `ghostty_surface_refresh`/`draw` on input and on `GHOSTTY_ACTION_RENDER`.
- Config: `~/.config/ghostty/config` is parsed by `ghostty_config_load_default_files` — themes, fonts, keybinds for *terminal* bindings come free. App-level bindings (splits, palette, jumps) live in Berth's own config and are checked first via `ghostty_config_key_is_binding` to avoid collisions.
- All C access is confined to `GhosttyHost.swift` + `GhosttySurfaceView.swift`. Nothing else in the app includes `ghostty.h`. This is the only file that changes on a Ghostty bump.

### 2.2 Shell — Swift/AppKit

Owns: windows, the split tree, tabs, sidebar, focus, key routing, notification presentation, menus, command palette. Owns *no* domain rules — it renders core state and forwards intents.

Split tree: custom, ~400 lines (n-ary node with orientation, children, weights; per-pane tab stack of surfaces). We do not vendor cmux's `bonsplit`; the tree is small and we need per-node metadata (unread counts, active agent, breadcrumbs). Persisted as a versioned JSON tree, not as a screenshot of UI state.

### 2.3 Domain core — `berth-core` (Rust)

Pure logic, no I/O, no threads, no AppKit. Compiled as a staticlib and wrapped with UniFFI. Contents:

- **Model**: `Workspace`, `Group`, `Pane`, `Surface`, `AgentSurface`, `Notification`.
- **Agent state machine**: legal transitions, staleness/TTL, attention derivation, dedupe.
- **Attention policy**: escalation ladder, mute rules, "latest unread" ordering.
- **Protocol codec**: request/response/event structs, validation, error codes, JSON Schema generation.
- **Router**: `handle(request, &mut State, &dyn SurfaceOps) -> Response` — a pure function with one injected capability trait.
- **Persistence**: session snapshot schema, schema version + migration, journal replay for crash recovery.
- **Provider adapters**: a small data table (process names, argv shapes, resume templates, hook names) for claude-code, codex, opencode, custom. Not code per provider.

`SurfaceOps` (UniFFI callback interface, implemented by Swift) is the only way core touches the outside world:

```rust
trait SurfaceOps {
    fn spawn(&self, req: SpawnRequest) -> Result<Spawned, CoreError>;
    fn send_text(&self, surface_id: &str, text: &str) -> Result<(), CoreError>;
    fn send_key(&self, surface_id: &str, key: &str, mods: &[String]) -> Result<(), CoreError>;
    fn read_text(&self, surface_id: &str, scope: ReadScope, max_bytes: usize) -> Result<Readback, CoreError>;
    fn close(&self, surface_id: &str) -> Result<(), CoreError>;
    fn focus(&self, surface_id: &str) -> Result<(), CoreError>;
    fn resize(&self, surface_id: &str, cols: u16, rows: u16) -> Result<(), CoreError>;
    fn pid_info(&self, surface_id: &str) -> Result<PidInfo, CoreError>; // fg pid, comm, argv
    fn notify_os(&self, n: &Notification) -> Result<(), CoreError>;
}
```

`spawn` is on the shell's side because Ghostty owns the PTY: the core decides
*that* a surface exists and what it runs, the shell decides *how*.

### 2.3.1 The FFI boundary

`berth-ffi` (crate `berth-ffi`, staticlib `libberth_ffi.a`) exposes one object:

```rust
Engine::new(host: Arc<dyn HostOps>)
Engine::dispatch(request_json) -> response_json   // the whole protocol
Engine::drain_events() -> ndjson                  // queued events
Engine::snapshot() / restore(json)                // session persistence
Engine::sweep_stale(ttl_ms) -> [agent_id]         // attention staleness
```

Deliberately JSON-shaped: Swift never learns the domain types, so the protocol
remains the single schema and there is no second set of generated structs to
keep in sync. The typed direction is `HostOps` (callbacks into Swift), because
that is the portability seam. Bindings are generated by `scripts/bindings.sh`,
committed, and verified for drift in CI.

This trait is also the **portability seam**: a future Rust shell on Linux implements it with its own PTY + libghostty-vt renderer, and `berth-core` does not change.

### 2.4 Protocol server (Swift)

A POSIX `AF_UNIX` socket at `berth.sock`, `0600`, NDJSON framing, one serial queue per connection (`SocketServer.swift`). Each connection is greeted with a `hello` event, then bytes → auth check (`SocketAuth.swift`) → `Engine.dispatch` → bytes. The transport keeps a bounded replay ring (`EventRing.swift`, last 1024 events) so `events.subscribe {"since": N}` can replay or report a `gap`; per-connection type filters and a monotonic `seq` cursor are enforced there. Request lines over 8 MiB close the connection as `payload_too_large`; a 4 MiB outbound backlog disconnects a stalled subscriber instead of growing without bound. After each request the handler drains the core's queue and fans the lines out to subscribed clients (`CoreHandler.swift`).

### 2.5 CLI — `berth` (Rust, no UI deps)

Thin client over the same protocol: `berth ls [--json]`, `berth send`, `berth send-key`, `berth read`, `berth split`, `berth events [--since N]`, `berth snapshot`, `berth notify`, `berth run -- <cmd>`. It authenticates with `BERTH_TOKEN` (inherited by children) or the token file. Also the binary invoked by editor hooks (`berth agent-set --state waiting_input`), so hook latency ≈ one socket round trip.

### 2.6 Agent pipeline

Agents are a core concern, not a shell decoration. `berth-core` owns the
provider table, the `AgentSurface` state machine, attention derivation, stale
sweeps, and notification dedupe. The Swift shell contributes three things:

- **Passive detection**: every two seconds the host asks the core to sniff each
  unclaimed surface's foreground process (`agent.detect` → `HostOps.pid_info`).
  A recognized `claude`/`codex`/`opencode` process creates an `unknown` agent,
  which is enough for a badge and a restore target.
- **Rendering**: the sidebar picks one primary agent per workspace (highest
  attention, then newest), draws its provider glyph, state, detail and progress,
  and sorts attention-first. A pane whose agent newly needs input or errors gets
  a 1.2 s accent ring; the focused surface is skipped.
- **Attention routing**: `⌘⇧U` jumps to the newest attention item and repeats
  through the list; `⌘I` opens the unread notification list as a palette. macOS
  banners are suppressed for the workspace being viewed.

Restore is agent-aware: `store::restore` replays each agent's saved argv/cwd,
injects `BERTH_AGENT_ID`, and leaves the surface in the tree with an `errored`
agent (`detail: "resume failed"`) if the provider cannot be resurrected. It
never silently substitutes a bare shell for a failed resume.

## 3. Concurrency rules

- **Main thread**: AppKit, Ghostty surfaces, Metal. Never blocked on the socket.
- **Core actor**: serial queue owning `State` (a `Mutex` inside Rust; UniFFI calls are synchronous). All mutations go through it.
- **Socket I/O**: own thread(s). Requests enqueue onto the core actor; the response is delivered back to the connection's thread.
- **Render path is sacred**: no core call, allocation, or lock is allowed inside `drawRect`/`ghostty_surface_draw`. Events are coalesced and flushed once per runloop turn.

## 4. Data model

```
Window (n per app)
 └─ Workspace (sidebar row; group, color, icon, unread, agents)
     └─ PaneNode tree (split or leaf)
         └─ Leaf: [Surface tab …] active index
             └─ Surface
                 ├─ TerminalHandle (ghostty_surface_t, pty, pid, tty)
                 ├─ LiveMeta { title, cwd, branch, ports, progress, bell }
                 └─ AgentRef ──► AgentSurface (0..1 per surface)
```

`Workspace` and `Surface` ids are stable and opaque (`ws_7f3a`, `srf_91c2`, `agt_44de`), minted by core, injected into child env (`BERTH_WORKSPACE_ID`, `BERTH_SURFACE_ID`, `BERTH_SOCKET_PATH`, `BERTH_AGENT_ID` when applicable). `TERM_PROGRAM=ghostty`, `TERM=xterm-ghostty` are kept — we are Ghostty-compatible on purpose.

## 5. Persistence

- `~/Library/Application Support/Berth/session.json` — versioned snapshot: windows, workspaces, tree, tabs, cwd, agent resume payloads. Written debounced (500 ms) and on quit.
- `journal.ndjson` — append-only structural changes since last snapshot; replayed on launch after a crash. Snapshot truncates journal.
- No terminal contents are persisted. Restore = new PTY in the saved cwd + agent resume argv; scrollback restore is explicitly not promised.

## 6. Repo layout

```
berth2/
  README.md
  docs/                      00 teardown · 01 vision · 02 architecture · 03 protocol · 04 roadmap
  vendor/ghostty/            git submodule, pinned tag
  app/                       Swift package + Xcode project
    Sources/BerthApp/        @main, AppDelegate, menus, palette
    Sources/BerthUI/         sidebar, split tree, tabs, notifications
    Sources/GhosttyKit/      C adapter: GhosttyHost, GhosttySurfaceView
    Sources/BerthCore/       UniFFI-generated Swift (do not edit by hand)
    Sources/Berth/BerthCoreHost.swift  HostOps implementation (Ghostty/AppKit)
    Sources/BerthDaemon/     socket server, subscriptions, framing
  core/                      cargo workspace
    crates/berth-core/       model, state, policy, codec, persistence
    crates/berth-ffi/        UniFFI scaffolding, SurfaceOps trait
    crates/berth-cli/        `berth` binary
  scripts/                   bootstrap, ghostty build, ffi gen, dev-sign
```

## 7. Decisions

| # | Decision | Rationale |
| --- | --- | --- |
| A1 | GhosttyKit (full Ghostty app lib), not libghostty-vt | vt has no renderer; writing one is the most expensive, least differentiating work |
| A2 | Single process, no PTY daemon | `ghostty_surface_new` owns PTYs; a daemon cannot feed GhosttyKit |
| A3 | Rust core, Swift shell | portability of the durable logic + native rendering where it matters |
| A4 | NDJSON JSON-RPC, bidirectional | human-debuggable, `jq`-friendly, trivial framing, events on the same channel |
| A5 | Own split tree, not bonsplit | ~400 lines, needs per-node metadata, avoids vendored UI divergence |
| A6 | No webview anywhere | text quality and event-loop control are the product |
| A7 | MIT, no GPL code | reuse and future portability without licensing ceilings |
| A8 | Provider adapters as data | agent CLIs change monthly; a table is cheaper than code paths |
| A9 | Restore from explicit resume payloads | deterministic; no evidence-file heuristics |

## 8. Rejected alternatives

- **All-Rust UI now (Rune/GPUI/egui).** Requires either an immature embedding story for a Metal terminal view or writing a terminal renderer. Rune is pre-release, takes no PRs, and cannot host a native `NSView`. Revisit for the Linux shell, where libghostty-vt + our own renderer is mandatory anyway.
- **Tauri/webview.** Text rendering and input latency are the two things users judge a terminal on; both are the webview's weakest points.
- **Go.** No mature native GPU UI; you end up on webview or Fyne.
- **libghostty-vt + custom renderer on macOS.** Best long-term ceiling, worst v1 schedule. Deferred, not dismissed.

## 9. Performance budgets

| Path | Budget |
| --- | --- |
| Cold start → first interactive frame (restored 3 workspaces) | ≤ 150 ms |
| Key input → paint | ≤ 1 frame (16.7 ms), no main-thread core calls |
| Socket round trip (local, small request) | ≤ 2 ms p99 |
| Idle CPU, 8 panes, 1 hidden window | < 1 % |
| RSS, 8 panes × 10k scrollback | ≤ 1.2 GB |

## 10. Operational risks

- Ghostty C API drift → pin tag, adapter file, upgrade checklist, smoke test suite (open 8 panes, run vim/htop/ssh, resize, close).
- Zig toolchain on macOS dev machines → `scripts/bootstrap` pins a Zig version and fails loudly.
- Zombie PTYs on crash → Ghostty owns children; on abnormal exit `request_close` on all surfaces, and the journal marks a dirty shutdown for restore.
- Code signing for local dev → ad-hoc signing only; no notarization in v1.
