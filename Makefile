# Berth — developer entry points.
# macOS for the app; the Rust core builds and tests anywhere cargo does.

SHELL := /bin/bash
CONFIG ?= release
ZIG_FLAGS := -Doptimize=ReleaseFast -Dapp-runtime=none -Demit-xcframework=true \
             -Demit-macos-app=false -Dxcframework-target=native

.PHONY: help bootstrap core-lib bindings build app run cli acceptance test test-core fmt fmt-check clean engine-clean

help:
	@echo "make bootstrap   fetch pinned Ghostty submodule + build GhosttyKit (macOS)"
	@echo "make core-lib    build libberth_ffi.a for this Mac"
	@echo "make bindings    regenerate UniFFI Swift bindings (commit them)"
	@echo "make build       swift build ($(CONFIG))"
	@echo "make app         assemble + ad-hoc sign build/Berth.app"
	@echo "make run         launch the app"
	@echo "make cli         build the berth CLI at core/target/release/berth"
	@echo "make acceptance  run the M2 protocol check against the running app"
	@echo "make test        swift tests (no window)"
	@echo "make test-core   cargo test --workspace (any platform)"
	@echo "make fmt         rustfmt + swift-format (if present)"
	@echo "make clean       remove build products (keeps submodule)"

bootstrap:
	./scripts/bootstrap.sh

core-lib:
	./scripts/core-lib.sh

bindings:
	./scripts/bindings.sh

# The app links libberth_ffi.a, so it must exist before SwiftPM evaluates.
build: core-lib
	swift build -c $(CONFIG) --package-path app

app: build
	./scripts/bundle.sh $(CONFIG)

run: app
	open build/Berth.app

cli:
	cd core && cargo build --release -p berth-cli

acceptance: cli
	./scripts/m2-acceptance.sh

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
