# cmux teardown + route draft

Working notes for **Berth** — a native terminal workspace for agentic coding, built on the
Ghostty engine. This file captures what cmux actually is (as of Nov 2026), what is worth
copying, what is worth beating, and the recommended technical route.

---

## 1. The idea in one paragraph

cmux is a native macOS terminal (Swift + AppKit) whose renderer is Ghostty's. It adds the
three things Ghostty deliberately does not: **a persistent vertical-tab sidebar with
per-workspace live metadata**, **an attention system** (pane rings, badges, notification
panel) aimed at agents that block on input, and **programmability** (a CLI plus a Unix
socket that can create/move/type into panes and drive an embedded browser). Its stated
philosophy ("The Zen of cmux") is explicitly anti-framework: it is a set of primitives,
not an orchestrator. That framing is why it spread — it does not tell you how to run agents,
it just makes running many of them legible.

## 2. Feature inventory

### Shell / layout
- Window → Workspace (sidebar entry) → Pane (split region) → Surface (tab in a pane) → Panel (terminal | browser).
- Vertical sidebar tabs; optional horizontal tabs; arbitrary nested splits (right/down).
- Workspace **groups**: collapsible named sections anchored to an owner workspace, with icon/color/pin.
- Right sidebar (secondary panel), TextBox, Dock, Task Manager.
- Command palette; per-project **custom commands** in `cmux.json`; extensive rebindable shortcuts.
- Reads `~/.config/ghostty/config` for theme/fonts/colors; own settings in `~/.config/cmux/cmux.json`.

### Sidebar metadata (the actual killer feature)
Per workspace, live: git branch, linked PR number/status, cwd, listening ports, latest
notification text, agent icon, unread badge. Plus a **scriptable metadata surface**:
`set-status` (icon/color/priority pill), `set-progress` (0..1 bar), `log` (info/progress/
success/warning/error entries), `sidebar-state` dump.

### Attention system
- Fires on OSC `9`, `99`, `777`, on `cmux notify`, and on agent hooks.
- Escalation: pane ring → sidebar unread badge → notification popover (⌘I) → macOS desktop
  notification → optional iOS push.
- ⌘⇧U jumps to most recent unread.

### Agent integrations
- Any TUI agent works (it is a terminal). Deep integrations exist for Claude Code (incl.
  `claude-teams` → teammates become native splits), oh-my-opencode / codex / pi / claudecode.
- Per-agent hooks report turn start/stop/needs-input into the sidebar.
- **Session restore**: windows, workspaces, panes, cwd, scrollback, and *agent sessions*
  (resume Claude Code / Codex / OpenCode) survive relaunch and reboot.
- Agent hibernation, agent inbox, journals, resume-evidence heuristics, PR/port tracking.

### Everything else
- In-app browser pane (WKWebView) with an agent-browser-derived API: accessibility-tree
  snapshot, element refs, click, fill, JS eval, console/network reads.
- SSH workspaces (`cmux ssh user@host`), remote tmux mirroring, a `cmux` TUI.
- Cookie/history import from 20+ browsers.
- Sparkle auto-update, Homebrew cask, many locales.
- Paid tiers (not part of the OSS core): iOS companion, cloud VMs, CodeRouter.

## 3. Integration surface (the part worth standardizing)

- Socket: `/tmp/cmux.sock` (override `CMUX_SOCKET_PATH`), one newline-terminated JSON
  request per connection: `{"id","method","params"}` → `{"id","ok","result"}`.
- Methods: `workspace.{list,create,select,current,close}`, `surface.{list,split,focus,
  send_text,send_key}`, `pane.surfaces`, `notification.{create,list,clear}`,
  `system.{ping,capabilities,identify}`, plus legacy string-form sidebar ops.
- Access modes: off / "cmux processes only" (checks process ancestry) / allowAll (env only).
- Injected env: `CMUX_WORKSPACE_ID`, `CMUX_SURFACE_ID`, `TERM_PROGRAM=ghostty`,
  `TERM=xterm-ghostty`.
- **Gap:** request/response only. No event stream, no subscriptions, no typed agent state.
  Automation must poll.

## 4. Data model

| Level | Meaning | Created by | Identity |
|---|---|---|---|
| Window | macOS window, own sidebar | ⌘⇧N | — |
| Workspace | sidebar entry ("tab" in UI) | ⌘N | `CMUX_WORKSPACE_ID` |
| Pane | split region | ⌘D / ⌘⇧D | pane id |
| Surface | tab within a pane | ⌘T | `CMUX_SURFACE_ID` |
| Panel | terminal or browser content | automatic | internal |

Naming inconsistency (UI says "tab", API says "workspace") is a real usability tax worth
fixing in a clone.

## 5. Implementation reality

- Swift 6 + AppKit + Xcode project (no Electron), SwiftUI only in places.
- Ghostty consumed as a **git submodule fork** (`manaflow-ai/ghostty`) + `GhosttyKit`
  (Zig-built static lib) bridged through `cmux-Bridging-Header.h`. Vendored split layout
  engine (`vendor/bonsplit`). Sparkle for updates.
- Licence: GPL (README: "free and GPL-licensed"; GitHub reports NOASSERTION).
- Scale: ~28.7k stars; repo ~9 GB (assets, vendor, tests); `Sources/` alone has thousands of
  files (`AgentHibernation/`, `AgentInbox/`, `AgentJournal*`, `AgentRestore*`, …).
  **Feature parity is not a realistic v1 goal.** A clone should pick 20% of this surface.

## 6. Where the openings are

1. **UI taste.** The most-quoted criticism in the wild is "it's a bit ugly / functions carry
   it". Ghostty-grade rendering + a sidebar that is actually designed (spacing, typography,
   density, motion, dark/light) is a differentiator, not cosmetics.
2. **Agent state is inferred, not modeled.** cmux bolts lifecycle onto terminals via hooks
   and journals. A first-class **typed surface** (`kind: agent`, provider, state machine:
   `idle → working → needs_input → done/failed`, plus the actual question text, tool calls,
   cost, model) is a cleaner contract for both UI and automation.
3. **No event stream.** Add subscriptions (`agent.state_changed`, `pane.output`, `pr.updated`)
   so orchestrators react instead of polling. This also makes remote/mobile trivial.
4. **Security model.** Ancestry checks are weak; per-surface capability tokens + scoped,
   revocable sessions are better.
5. **Licence / portability.** GPL + macOS-only is a ceiling. MIT core + a documented protocol
   invites a Linux shell, editor plugins, and third-party clients.
6. **Restore is heuristic.** A tiny, explicit provider contract ("here is how to resume this
   session") beats reverse-engineering agent CLIs.

## 7. Engine reality check (Nov 2026)

**libghostty-vt 1.0.0** shipped **Nov 6 2026**: MIT, stable C ABI (semver, symbol versioning,
ABI CI), tarballs/CMake/pkg-config, XCFrameworks for macOS/iOS, bindings for Swift/Rust/Go/JS/
Python/.NET/Elixir/Dart/Java, docs at lib.ghostty.org. It is `ghostty/vt.h`:
`ghostty_terminal_new` / `ghostty_terminal_vt_write` / formatters, screen + scrollback + cursor
state, no threads, custom allocator.

**It is emulation only — no renderer.** Mitchell's own footnote: performance "means parsing
throughput and memory usage, since those are the parts libghostty-vt provides". Everything
users *see* (font shaping/ligatures, glyph atlas, GPU blitting, images, selection, IME, link
detection, scrollback perf) comes from Ghostty's app code, which is MIT but not a library.

| | A. Vendor Ghostty (GhosttyKit) | B. libghostty-vt + own renderer |
|---|---|---|
| Emulation | Ghostty, battle-tested | Ghostty, battle-tested, **stable ABI** |
| Rendering | Metal, done, excellent | must build: shaping, atlas, GPU, images, IME, selection |
| Build | Zig static lib + Xcode, pinned fork, ~GB checkout | normal SwiftPM/Xcode dependency |
| Upgrade path | rebase a fork (API unstable, expect breakage) | semver bumps |
| Effort to first pixels | days | months |
| Portability | macOS app code is not reusable | same core can back Linux/Windows/web shells |

cmux chose A. It is the only sane choice for a solo dev who wants a *usable macOS app this
quarter*. Choose B only if the plan is "engine + protocol company", not "terminal app".

## 8. Recommended route (draft)

**Berth v1 — macOS, Swift + AppKit, vendored GhosttyKit.** Native, fast, no Electron, and it
inherits Ghostty config/theme/shell-integration. Differentiate on the things cmux left thin:
typed agent surfaces, an event-driven protocol, and actual visual design. Keep the GPL'd cmux
out of the tree entirely (clean-room; MIT for our code, MIT for Ghostty).

Shape:
- `BerthKit` (SPM package, no AppKit): session tree (window/workspace/pane/surface), agent
  state machine, notification model, protocol codecs, persistence. Testable, headless.
- `BerthApp` (AppKit shell): sidebar, split layout, `MTKView` Ghostty surface, palette,
  notifications, settings.
- `berth` CLI: thin client over the socket (same binary surface as the app).
- `berthd`-lite: the app *is* the daemon for v1; socket is the contract so a headless daemon
  can replace it later.

Non-goals for v1: browser pane, SSH, remote tmux, iOS, cloud, teams orchestration, market
economy of skills. They are v2/v3, and the protocol is designed so they can land without
reshaping the core.

Open decisions are tracked as questions to the owner before this becomes `01-vision.md` /
`02-architecture.md` / `03-roadmap.md`.
