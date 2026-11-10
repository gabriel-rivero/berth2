# Berth

A native macOS terminal workspace for running coding agents side by side — built on Ghostty's renderer, with a typed agent lifecycle and a push-based protocol.

Status: M0 scaffold. The engine is pinned, the app shell boots a single Ghostty terminal, and the portable core (agent lifecycle, protocol, persistence) is implemented and tested on Linux. Full key routing and the core↔app bridge are the next two tasks.

```sh
make test-core    # rust core, any platform
make bootstrap    # macOS: build pinned GhosttyKit
make run          # macOS: bundle + launch
```


## The idea in one paragraph

cmux proved the shape (sidebar of workspaces next to a split grid of terminals) but models agents as free-form status strings that integrations push and heuristics guess. Berth keeps the shape and changes the foundation: an agent is a **typed object** with a lifecycle, a resume payload, and an attention level; every change is an **ordered event** on a subscribable socket, so editors, dashboards and hooks never poll. Terminals stay plain terminals — nothing is required to run a normal shell or a normal agent CLI.

## Documents

| Doc | What it covers |
| --- | --- |
| [docs/00-cmux-teardown.md](docs/00-cmux-teardown.md) | What cmux is, its features, socket API, integrations, and where it is weak |
| [docs/01-vision.md](docs/01-vision.md) | User, principles, non-goals, differentiators, definition of done |
| [docs/02-architecture.md](docs/02-architecture.md) | Process model, layers, Ghostty integration, Rust core, repo layout, ADRs, budgets |
| [docs/03-protocol.md](docs/03-protocol.md) | Socket transport, auth, methods, events, agent schema and state machine |
| [docs/04-roadmap.md](docs/04-roadmap.md) | M0–M4 milestones with acceptance criteria, post-v1, kill criteria |
| [docs/05-build.md](docs/05-build.md) | Toolchain, bootstrap, smoke checklist, scaffold gaps, Ghostty upgrades |

## Repo layout

```
app/            Swift 6 + AppKit shell, hosts GhosttyKit (Package.swift + Sources/Berth)
core/           cargo workspace: berth-core (portable domain) + berth-cli (`berth`)
vendor/ghostty  git submodule, pinned to v1.4.0
scripts/        bootstrap.sh (engine) · bundle.sh (ad-hoc signed .app)
docs/           00 teardown · 01 vision · 02 architecture · 03 protocol · 04 roadmap · 05 build
```


## Route chosen (and why)

- **Shell: Swift 6 + AppKit, hosting vendored GhosttyKit.** `ghostty_surface_new` makes Ghostty own the PTY *and* the renderer; there is no external-PTY mode. libghostty-vt 1.0 has no renderer at all. Any route that isn't "host Ghostty's app library" means writing Metal text rendering from scratch — the most expensive and least differentiating part of this product.
- **Domain core: Rust (`berth-core`), UniFFI'd into the app.** Model, agent state machine, attention policy, protocol codec, persistence, provider adapters — all portable, all headless-testable, zero UI I/O.
- **Protocol: NDJSON JSON-RPC over a unix socket, bidirectional with subscriptions.** The durable contract; the future Linux/Windows shell and every integration target it.
- **Rejected:** all-Rust GPU UI now (Rune is pre-release, no PRs, cannot embed a native Metal terminal view); Tauri (text quality, input latency); Go (no mature native GPU UI).
- **License: MIT.** No GPL surface inherited from cmux.

## Non-goals for v1

Browser panes, SSH, iOS, cloud VMs, teams, plugins, TUI, Windows/Linux builds.
