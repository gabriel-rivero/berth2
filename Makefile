# Berth — developer entry points.
# macOS for the app; the Rust core builds and tests anywhere cargo does.

SHELL := /bin/bash
CONFIG ?= release
ZIG_FLAGS := -Doptimize=ReleaseFast -Dapp-runtime=none -Demit-xcframework=true

.PHONY: help bootstrap build app run test test-core fmt fmt-check clean dist

help:
	@echo "make bootstrap   fetch pinned Ghostty submodule + build GhosttyKit (macOS)"
	@echo "make build       swift build ($(CONFIG))"
	@echo "make app         assemble + ad-hoc sign build/Berth.app"
	@echo "make run         launch the app"
	@echo "make test        swift tests (no window)"
	@echo "make test-core   cargo test --workspace (any platform)"
	@echo "make fmt         rustfmt + swift-format (if present)"
	@echo "make clean       remove build products (keeps submodule)"

bootstrap:
	./scripts/bootstrap.sh

build:
	swift build -c $(CONFIG) --package-path app

app:
	./scripts/bundle.sh $(CONFIG)

run: app
	open build/Berth.app

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
