# Berth — developer entry points.
# The Rust workspace builds and tests on Linux, macOS and Windows; the macOS
# app targets (build/app/run) need macOS + Xcode.

SHELL := /bin/bash
CONFIG ?= release
ZIG_FLAGS := -Doptimize=ReleaseFast -Dapp-runtime=none -Demit-xcframework=true \
             -Demit-macos-app=false -Dxcframework-target=native

.PHONY: help bootstrap fetch-engine core-lib bindings build app run cli headless gui gui-app gui-smoke gui-notarize perf-trace acceptance m3 schema icon test test-core fmt fmt-check clean engine-clean

help:
	@echo "make bootstrap   fetch pinned Ghostty submodule + build GhosttyKit (macOS)"
	@echo "make fetch-engine download the CI-built GhosttyKit (macOS, no Xcode needed)"
	@echo "make core-lib    build libberth_ffi.a for this Mac"
	@echo "make bindings    regenerate UniFFI Swift bindings (commit them)"
	@echo "make build       swift build ($(CONFIG))"
	@echo "make app         assemble + ad-hoc sign build/Berth.app"
	@echo "make run         launch the app"
	@echo "make cli         build the berth CLI at core/target/release/berth"
	@echo "make headless    build the headless shell at core/target/release/berth-headless"
	@echo "make gui         build the portable window at core/target/release/berth-gui"
	@echo "make gui-app     wrap it in build/BerthGui.app (macOS, ad-hoc signed)"
	@echo "make gui-notarize sign with DEVELOPER_ID and notarize (macOS)"
	@echo "make perf-trace  record a symbolized startup trace (macOS)"
	@echo "make icon        regenerate the .icns, .png and .ico from packaging/berth.svg"
	@echo "make gui-smoke   render the window under Xvfb and exit (headless check)"
	@echo "make acceptance  run the M2 protocol check against the running app"
	@echo "make m3          run the M3 agent check against the running app"
	@echo "make test        swift tests (no window)"
	@echo "make test-core   cargo test --workspace (Linux, macOS, Windows)"
	@echo "make fmt         rustfmt + swift-format (if present)"
	@echo "make clean       remove build products (keeps submodule)"

bootstrap:
	./scripts/bootstrap.sh

fetch-engine:
	./scripts/fetch-engine.sh

core-lib:
	./scripts/core-lib.sh

bindings:
	./scripts/bindings.sh

# The app links libberth_ffi.a, so it must exist before SwiftPM evaluates.
build: core-lib
	swift build -c $(CONFIG) --package-path app

schema:
	cd core && cargo run -q -p berth-cli -- schema > ../docs/schema/berth.schema.json

app: build
	./scripts/bundle.sh $(CONFIG)

run: app
	open build/Berth.app

cli:
	cd core && cargo build --release -p berth-cli

headless:
	cd core && cargo build --release -p berth-headless

gui:
	cd core && cargo build --release -p berth-gui

gui-app: gui cli
	./scripts/bundle-gui.sh release

perf-trace:
	./scripts/perf-trace.sh

icon:
	./scripts/make-icon.sh

gui-notarize: gui-app
	./scripts/notarize-gui.sh

gui-smoke: gui
	./scripts/gui-smoke.sh

acceptance: cli
	./scripts/m2-acceptance.sh

m3: cli
	./scripts/m3-acceptance.sh

test:
	swift test --package-path app

test-core:
	cd core && cargo test --workspace

check: test-core test

fmt:
	cd core && cargo fmt --all
	@command -v swift-format >/dev/null && swift-format format -i -r app/Sources app/Tests || echo "swift-format not installed (skipped)"

fmt-check:
	cd core && cargo fmt --all -- --check
	@command -v swift-format >/dev/null && swift-format lint -r app/Sources app/Tests || echo "swift-format not installed (skipped)"

clean:
	rm -rf build app/.build
	cd core && cargo clean

# Rebuild the engine from scratch after a submodule bump.
engine-clean:
	rm -rf vendor/ghostty/macos/GhosttyKit.xcframework vendor/ghostty/zig-out vendor/ghostty/.zig-cache
