# Berth — build

Two toolchains, two halves. The Rust core builds anywhere; the app needs macOS.

## Prerequisites (app)

| Need | Version | Notes |
| --- | --- | --- |
| macOS | 14+ | Apple Silicon; Metal required |
| Xcode | 16+ | Swift 6 toolchain, macOS SDK |
| Zig | 0.17.x | `vendor/ghostty/build.zig.zon` pins the minimum; `scripts/bootstrap.sh` checks |
| Rust | stable | the portable core; `make core-lib` builds the static library |
| git | any | submodule handling |

No Homebrew packages required. Nothing is installed system-wide except Zig.

## First run

```sh
git submodule update --init --recursive   # or: make bootstrap
make bootstrap                            # builds GhosttyKit.xcframework (minutes)
make build                                # builds libberth_ffi.a, then swift build
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
- [ ] `vim` opens, arrows/Esc work, `:q` exits
- [ ] `ctrl-c` interrupts a `sleep 100`, `ctrl-r` searches history, `ctrl-a`/`ctrl-e` move the line
- [ ] `⌘K` clears (Ghostty binding wins over the menu), `⌘Q` still quits
- [ ] `htop` renders colors and boxes without tearing
- [ ] `seq 1 100000 | less` scrolls smoothly; no dropped frames while tailing a log
- [ ] Resize the window: no artifacts, no ghost cells, grid reflows
- [ ] Move between Retina and non-Retina displays: text stays crisp
- [ ] Paste with `⌘V`, copy with `⌘C` (selection mouse-drag)
- [ ] Wheel scrolls back through `seq 1 5000`; drag-select highlights and copies
- [ ] `htop` responds to clicks and wheel (terminal mouse reporting)
- [ ] On a Spanish layout, `´` then `a` types `á`; emoji picker inserts text
- [ ] Close the window: child process is gone (`pgrep -P` empty), no zombie
- [ ] Idle CPU < 1 %, RSS sane for one pane
- [ ] `berth ping`, `berth ls`, `berth agents` answer from a terminal in the app
- [ ] `berth new -- sh` opens a second window; `berth surface read_text` returns that screen

Until every line passes, M0 is not done.

## Known gaps (deliberate, ordered)

1. **Socket token auth.** The socket is `0600` in the user's Application Support
   directory, but the token handshake described in docs/03-protocol.md §2 is not
   implemented; access is "any process running as you".
2. **Sidebar.** Workspaces exist in the core and each one is a window, but there
   is no workspace list, no git branch, no unread badges.
3. **Per-pane tabs.** A pane holds one surface; the model already supports a
   stack, the UI does not expose it.
4. **Session journal.** Persistence is a snapshot on quit; a crash loses the last
   structural change. The journal in docs/02 §2.3 is not written yet.
5. **Geometric focus.** ⌘] / ⌘[ cycle surfaces in tree order; arrow-key
   navigation between panes needs pane rectangles and lands with the sidebar.

Done since the scaffold: key routing, IME and dead keys, mouse (wheel
scrollback, drag selection, terminal mouse reporting, right/middle buttons),
core bridge over UniFFI, clipboard with paste confirmation, OSC reporting
(title, cwd, progress, bell, notifications, URLs), split tree with draggable
dividers, and ⌘D/⌘⇧D/⌘W.

## Upgrading Ghostty

```sh
cd vendor/ghostty && git fetch --tags && git checkout v1.4.x
cd ../.. && make engine-clean && make bootstrap
```

Then run the M0 smoke checklist. Cost budget: one day per bump, all C access confined to `GhosttyHost.swift` / `GhosttySurfaceView.swift` / `BerthCoreHost.swift`. If a bump breaks the adapter twice in a row, invoke the kill criterion in docs/04-roadmap.md.

## Rust core

```sh
cd core
cargo test --workspace        # 40 tests, no window needed
cargo run -p berth-cli -- ls  # needs a running Berth (socket in Application Support)
```

After changing anything in `core/crates/berth-ffi`, regenerate the committed
bindings with `make bindings`; CI fails if they drift.

`core/` has no platform dependencies and no I/O in `berth-core`; the only impure surface is the `SurfaceOps` trait, implemented by the shell.

## Troubleshooting

| Symptom | Cause |
| --- | --- |
| `artifact path does not exist: GhosttyKit.xcframework` | run `make bootstrap` |
| `ghostty_init` returns non-zero | resources not bundled; check `Berth.app/Contents/Resources/ghostty`, or set `GHOSTTY_RESOURCES_DIR` |
| blank window, no prompt | surface never created — check `createSurface` logs, view size 0 at first layout |
| app crashes on quit | a surface freed after its layer; free surfaces before `ghostty_app_free` |
| `Library not loaded: /Users/.../libberth_ffi.dylib` | the core was linked as a dylib; `Package.swift` must name `libberth_ffi.a` by path (CI checks the bundle with `otool -L`) |
| `zig: error: no module named ...` | wrong Zig version; `make bootstrap` prints the required one |
