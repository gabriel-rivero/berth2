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
- [ ] `berth new -- sh` opens a second workspace; `berth surface read_text` returns that screen

Until every line passes, M0 is not done.

## M1 restore checklist (run on the Mac)

The release criterion is: three workspaces, six panes, quit, relaunch, and land
exactly where you left off including each shell's cwd.

1. Launch the app and confirm the sidebar and one terminal appear.
2. Click the sidebar `+` twice to create workspaces 2 and 3. Rename them mentally
   (or move them) so they are distinguishable.
3. In the selected workspace press `⌘D` once and `⌘⇧D` once to make three panes.
   Switch to each workspace and do the same, ending with six panes total.
4. In every pane, `cd` to a unique directory and run `pwd`, for example:
   `cd /tmp/berth-w1-a && pwd`.
5. In one pane press `⌘T` and `cd` the new tab somewhere else. Leave a different
   tab active in another pane.
6. Select workspace 2, then quit with `⌘Q`.
7. Relaunch. Check:
   - [ ] the same three workspaces are in the sidebar, same order
   - [ ] the same window layout (three panes per workspace) is rebuilt
   - [ ] workspace 2 is selected, and the active tab in every pane is preserved
   - [ ] every shell starts in the cwd it had before quitting (`pwd`)
   - [ ] cwd survives a shell whose process had already exited
   - [ ] no visible startup hitch before the first prompt
8. Crash path: make a small change, wait one second, then `kill -9` the app.
   Relaunch and repeat the checks. A change made less than 500 ms before the
   kill may be missing; that is the documented debounce window.

Split divider positions are a known gap (docs/04 M5); do not fail the criterion
on those until weights are wired back.

## M2 protocol checkpoint (run on the Mac)

Start a release build, then from the repo root:

```sh
export PATH="$PWD/core/target/release:$PATH"   # or: make cli
berth ping
berth schema | head
berth new --name m2-check
berth active                                    # active surface id
make cli && ./scripts/m2-acceptance.sh
```

The script creates a workspace, splits twice, types into the deepest pane, reads the screen back, and runs two independent `events --since 0` clients. It fails unless both clients see identical NDJSON with strictly increasing `seq`. It cleans up the workspace on exit.

Auth check: in `local` mode (default), a mutating call without a token must fail:

```sh
printf '%s\n' '{"id":1,"method":"workspace.create","params":{"name":"nope"}}' \
  | nc -U "$HOME/Library/Application Support/Berth/berth.sock"
# → {"id":1,"ok":false,"error":{"code":"permission_denied",...}}
```

Read the token from `~/Library/Application Support/Berth/token` or use `$BERTH_TOKEN` inside a Berth terminal. Socket mode lives in `~/.config/berth/config.json` (`off`, `local`, `trusted`) and in Settings.

## Known gaps

These mirror the M5 pull-forward list in `docs/04-roadmap.md`; none should be
mistaken for missing architecture.

1. **Headless app-test mode.** The acceptance script needs the GUI app running;
   there is no windowless app loop for CI.
2. **Acceptance ergonomics.** Run `make cli && ./scripts/m2-acceptance.sh`; there
   is no `make acceptance` target, and `make cli ./scripts/m2-acceptance.sh`
   treats the script as a build target instead of running it.
3. **Generated JSON Schema.** `berth schema` prints the capability list; the
   generated `docs/schema/` promised by the protocol doc does not exist yet.
4. **Socket hardening.** Token auth is implemented; peer-UID checks, hiding
   `BERTH_TOKEN` from children, and non-bypassable `trusted` mode are not.
5. **Group management UI.** Groups render and collapse, and the protocol has
   `group.create/update/delete`, but there is no create/rename/color picker yet.
6. **Split weights.** The core stores per-split weights; the Swift shell rebuilds
   `NSSplitView` from the tree and does not feed weights back into the core, so
   divider positions are not restored across launches.
7. **Journal semantics.** `journal.ndjson` stores whole snapshots (newest wins)
   rather than replayable structural events, because events cannot recreate
   PTYs. A crash loses at most the last 500 ms debounce window.
8. **Geometric focus.** ⌘] / ⌘[ move between panes in tree order; rectangle-based
   arrow navigation is not implemented.
9. **Live font reload.** Font size applies on relaunch.

Done since the scaffold: key routing, IME and dead keys, mouse (wheel
scrollback, drag selection, terminal mouse reporting, right/middle buttons),
core bridge over UniFFI, clipboard with paste confirmation, OSC reporting
(title, cwd, progress, bell, notifications, URLs), split tree with draggable
dividers, sidebar with live metadata and drag reorder, per-pane tab stacks,
command palette, editable shortcuts, and session restore with cwd and active
tab.

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
