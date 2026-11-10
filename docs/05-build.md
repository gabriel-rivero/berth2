# Berth — build

Two toolchains, two halves. The Rust core builds anywhere; the app needs macOS.

## Prerequisites (app)

| Need | Version | Notes |
| --- | --- | --- |
| macOS | 14+ | Apple Silicon; Metal required |
| Xcode | 16+ | Swift 6 toolchain, macOS SDK |
| Zig | 0.17.x | `vendor/ghostty/build.zig.zon` pins the minimum; `scripts/bootstrap.sh` checks |
| git | any | submodule handling |

No Homebrew packages required. Nothing is installed system-wide except Zig.

## First run

```sh
git submodule update --init --recursive   # or: make bootstrap
make bootstrap                            # builds GhosttyKit.xcframework (minutes)
make build                                # swift build -c release
make app                                  # assembles + ad-hoc signs build/Berth.app
open build/Berth.app
```

`make bootstrap` runs, inside `vendor/ghostty`:

```sh
zig build -Doptimize=ReleaseFast -Dapp-runtime=none \
  -Demit-xcframework=true -Demit-macos-app=false \
  -Dxcframework-target=native install
```

which produces two things we consume:

- `vendor/ghostty/macos/GhosttyKit.xcframework` — static lib + `ghostty.h` + a clang module map, so Swift does `import GhosttyKit`.
- `vendor/ghostty/zig-out/share/ghostty/` — terminfo, shell integration, themes. Copied into `Berth.app/Contents/Resources/ghostty`.

Both are gitignored. `app/Package.swift` references the xcframework by path, so `swift build` fails until bootstrap has run.

`-Demit-macos-app=false` matters: Ghostty defaults `emit-xcframework` to also build its own `.app` through Xcode, which is slower and pins us to whatever Xcode version Ghostty's app currently compiles with. We only consume the library. `-Dxcframework-target=native` emits one architecture slice instead of a universal fat archive.

`-Dapp-runtime=none` is what selects Ghostty's GUI core library (the thing the Ghostty macOS app itself links) instead of the standalone desktop app. `-Demit-lib-vt` stays off: we are not building the renderer-less VT library.

## Everyday loop

```sh
make test-core   # cargo test --workspace — 35 tests, no window needed
make test        # swift tests (framing, helpers; nothing GPU/PTY)
make run         # bundle + launch
```

The core is where the product logic lives and it is fully headless-testable, so run `make test-core` on every change. The app tests stay thin on purpose; anything visual goes on the smoke list below.

## M0 smoke checklist (run on the Mac after every Ghostty bump)

- [ ] Window opens, cursor blinks, prompt appears (default shell)
- [ ] `vim` opens, arrows/Esc work, `:q` exits — **blocked until key routing lands** (see gaps)
- [ ] `htop` renders colors and boxes without tearing
- [ ] `seq 1 100000 | less` scrolls smoothly; no dropped frames while tailing a log
- [ ] Resize the window: no artifacts, no ghost cells, grid reflows
- [ ] Move between Retina and non-Retina displays: text stays crisp
- [ ] Paste with `⌘V`, copy with `⌘C` (selection mouse-drag)
- [ ] Close the window: child process is gone (`pgrep -P` empty), no zombie
- [ ] Idle CPU < 1 %, RSS sane for one pane

Until every line passes, M0 is not done.

## Known scaffold gaps (deliberate, ordered)

1. **Key routing.** `keyDown` currently forwards printable characters only via `ghostty_surface_text`, so Esc/Ctrl/arrows do not work. Fix: build `ghostty_input_key_s` (action/mods/consumed_mods/keycode/text/unshifted_codepoint/composing) → `ghostty_surface_key`, and consult `ghostty_config_key_is_binding` first so terminal bindings win over app bindings.
2. **Action callbacks.** `berthActionCallback` returns `false` for everything. Wire `SET_TITLE`, `PWD`, `PROGRESS_REPORT`, `RING_BELL`, `DESKTOP_NOTIFICATION`, `OPEN_URL`, `SHOW_CHILD_EXITED` into the core (docs/03-protocol.md §5).
3. **Clipboard.** Read/write confirm callbacks are stubs.
4. **IME.** `ghostty_surface_preedit` + `ime_point` are not called yet; no CJK input.
5. **Core bridge.** `SocketServer` runs with `StubHandler`; UniFFI (`core/crates/berth-ffi`) is M2. `berth ping` works end to end once the socket is started from `AppDelegate`.
6. **Split tree, sidebar, palette** — M1.

## Upgrading Ghostty

```sh
cd vendor/ghostty && git fetch --tags && git checkout v1.4.x
cd ../.. && make engine-clean && make bootstrap
```

Then run the M0 smoke checklist. Cost budget: one day per bump, all C access confined to `GhosttyHost.swift` / `GhosttySurfaceView.swift`. If a bump breaks the adapter twice in a row, invoke the kill criterion in docs/04-roadmap.md.

## Rust core

```sh
cd core
cargo test --workspace        # 35 tests
cargo run -p berth-cli -- ls  # needs a running app with a wired socket
```

`core/` has no platform dependencies and no I/O in `berth-core`; the only impure surface is the `SurfaceOps` trait, implemented by the shell.

## Troubleshooting

| Symptom | Cause |
| --- | --- |
| `artifact path does not exist: GhosttyKit.xcframework` | run `make bootstrap` |
| `ghostty_init` returns non-zero | resources not bundled; check `Berth.app/Contents/Resources/ghostty`, or set `GHOSTTY_RESOURCES_DIR` |
| blank window, no prompt | surface never created — check `createSurface` logs, view size 0 at first layout |
| app crashes on quit | a surface freed after its layer; free surfaces before `ghostty_app_free` |
| `zig: error: no module named ...` | wrong Zig version; `make bootstrap` prints the required one |
