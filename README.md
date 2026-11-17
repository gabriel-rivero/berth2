# Berth

A native macOS terminal workspace for running coding agents side by side — built on Ghostty's renderer, with a typed agent lifecycle and a push-based protocol.

Status: 0.8.4, M4 code complete; 0.9 closes the milestone with the focus-ownership fix and the Instruments trace ([docs/06-0.9.md](docs/06-0.9.md)). The daily-driver work is in: `berth hooks install` wires Claude Code hooks and an opencode plugin so agent state, tool detail, and the provider session id feed the M3 agent layer (hooks also make `claude --resume <id>` work across relaunch). M3 remains the agent layer: `AgentSurface` state machine with attention/notification dedupe, one-second passive foreground-process detection, process-exit → `errored` within the same tick, `berth run` registration with resume payloads, sidebar agent badges and attention-first ordering, pane attention rings, acknowledging `⌘⇧U` jump, `⌘I` attention panel, and agent-aware session restore. M4 hardening frees PTYs before Ghostty shuts down, validates session snapshots before restoring, rotates the crash journal, and records rolling breadcrumbs plus a main-thread stall watchdog under `~/Library/Logs/Berth/`. Real use surfaced three defects, all fixed: a split-resize feedback loop that crashed ⌘D (0.8.2), a detection deadlock that froze the app a second or two after launch (0.8.3), and a 1× display scale mismatch that drew the grid at half size (0.8.4). The only open M4 criterion is the two-week personal-use observation. M0–M2 remain the shell and authenticated NDJSON protocol spine, now with generated JSON Schema.

Download: [latest release](https://github.com/gabriel-rivero/berth2/releases) (macOS 14+, Apple Silicon, ad-hoc signed).

```sh
make test-core    # rust core + transport + server, any platform (Linux/macOS/Windows)
make bootstrap    # macOS + Xcode: build pinned GhosttyKit
make fetch-engine # macOS, Command Line Tools only: download the CI-built GhosttyKit
make run          # macOS: bundle + launch
```

The Rust half — `berth-core`, `berth-transport`, `berth-server`, `berth-headless` and the `berth` CLI — builds and tests on Linux, macOS and Windows. `berth-headless` is the first shell slice: real PTYs, a VT screen and the full protocol with no window (see [docs/07-headless-shell.md](docs/07-headless-shell.md)). The Swift/AppKit shell remains the macOS shell; Linux and Windows get their GUI shell on the same protocol (see [docs/05-build.md](docs/05-build.md#portable-core-linux-and-windows)).


## The idea in one paragraph

cmux proved the shape (sidebar of workspaces next to a split grid of terminals) but models agents as free-form status strings that integrations push and heuristics guess. Berth keeps the shape and changes the foundation: an agent is a **typed object** with a lifecycle, a resume payload, and an attention level; every change is an **ordered event** on a subscribable socket, so editors, dashboards and hooks never poll. Terminals stay plain terminals — nothing is required to run a normal shell or a normal agent CLI.

## Documents

| Doc | What it covers |
| --- | --- |
| [docs/00-cmux-teardown.md](docs/00-cmux-teardown.md) | What cmux is, its features, socket API, integrations, and where it is weak |
| [docs/01-vision.md](docs/01-vision.md) | User, principles, non-goals, differentiators, definition of done |
| [docs/02-architecture.md](docs/02-architecture.md) | Process model, layers, Ghostty integration, Rust core, repo layout, ADRs, budgets |
| [docs/03-protocol.md](docs/03-protocol.md) | Socket transport, auth, methods, events, agent schema and state machine |
| [docs/04-roadmap.md](docs/04-roadmap.md) | M0–M5 milestones with acceptance criteria, post-v1, kill criteria |
| [docs/05-build.md](docs/05-build.md) | Toolchain, bootstrap, smoke checklist, known gaps, Ghostty upgrades |
| [docs/06-0.9.md](docs/06-0.9.md) | 0.9 scope: focus ownership fix, Instruments trace, filed-bug queue |
| [docs/07-headless-shell.md](docs/07-headless-shell.md) | Linux/Windows shell spike: portable PTY host, VT readback, protocol acceptance |
| [docs/config.md](docs/config.md) | `config.json`, font/display scale, sidebar density, socket and env vars, logs |
| [CHANGELOG.md](CHANGELOG.md) | Release notes, including 0.8.4 |

## Repo layout

```
app/            Swift 6 + AppKit shell, hosts GhosttyKit (Package.swift + Sources/Berth)
core/           cargo workspace: berth-core (portable domain) · berth-transport (sockets/pipes) · berth-server (NDJSON server) · berth-headless (PTY/VT shell) · berth-cli (`berth`)
vendor/ghostty  git submodule, pinned to v1.4.0
scripts/        bootstrap.sh (engine) · fetch-engine.sh (CI-built engine) · bundle.sh (.app)
docs/           00 teardown · 01 vision · 02 architecture · 03 protocol · 04 roadmap · 05 build · 07 headless shell
```


## Route chosen (and why)

- **Shell: Swift 6 + AppKit, hosting vendored GhosttyKit.** `ghostty_surface_new` makes Ghostty own the PTY *and* the renderer; there is no external-PTY mode. libghostty-vt 1.0 has no renderer at all. Any route that isn't "host Ghostty's app library" means writing Metal text rendering from scratch — the most expensive and least differentiating part of this product.
- **Domain core: Rust (`berth-core`), UniFFI'd into the app.** Model, agent state machine, attention policy, protocol codec, persistence, provider adapters — all portable, all headless-testable, zero UI I/O.
- **Protocol: NDJSON JSON-RPC over a unix socket, bidirectional with subscriptions.** The durable contract; the future Linux/Windows shell and every integration target it.
- **Rejected:** all-Rust GPU UI now (Rune is pre-release, no PRs, cannot embed a native Metal terminal view); Tauri (text quality, input latency); Go (no mature native GPU UI).
- **License: MIT.** No GPL surface inherited from cmux.

## Non-goals for v1

Browser panes, SSH, iOS, cloud VMs, teams, plugins, TUI, Windows/Linux builds.
