#!/usr/bin/env bash
# Assemble a runnable, ad-hoc-signed Berth.app from the SwiftPM build.
# No notarization, no distribution concerns: this is a daily driver.
set -euo pipefail

cd "$(dirname "$0")/.."
CONFIG="${1:-release}"
APP="build/Berth.app"

if [[ ! -d vendor/ghostty/macos/GhosttyKit.xcframework ]]; then
  echo "bundle: GhosttyKit.xcframework missing — run scripts/bootstrap.sh first." >&2
  exit 1
fi

echo "==> swift build ($CONFIG)"
swift build -c "$CONFIG" --package-path app
BIN="$(swift build -c "$CONFIG" --package-path app --show-bin-path)/Berth"

VERSION="$(git describe --tags --abbrev=0 2>/dev/null | sed 's/^v//')"
VERSION="${VERSION:-0.0.0}"
BUILD="$(git rev-list --count HEAD 2>/dev/null || echo 1)"

echo "==> assembling $APP ($VERSION build $BUILD)"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/Berth"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Berth</string>
  <key>CFBundleDisplayName</key><string>Berth</string>
  <key>CFBundleIdentifier</key><string>dev.berth.app</string>
  <key>CFBundleExecutable</key><string>Berth</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${BUILD}</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST

# LaunchServices rejects placeholder or non-numeric versions with a useless
# error (often -47 / fBsyErr), so fail the bundle instead of shipping one.
if grep -q '\${' "$APP/Contents/Info.plist"; then
  echo "bundle: Info.plist still contains an unexpanded placeholder" >&2
  exit 1
fi
if [[ ! "$BUILD" =~ ^[0-9]+$ ]]; then
  echo "bundle: CFBundleVersion must be numeric, got '$BUILD'" >&2
  exit 1
fi

# Ghostty needs its runtime resources (terminfo, shell integration, themes).
if [[ -d vendor/ghostty/zig-out/share/ghostty ]]; then
  cp -R vendor/ghostty/zig-out/share/ghostty "$APP/Contents/Resources/ghostty"
else
  echo "bundle: warning: ghostty resources not found; shell integration will be missing" >&2
fi

echo "==> ad-hoc signing"
codesign --force --deep --sign - "$APP" >/dev/null

echo "ok: $APP"
