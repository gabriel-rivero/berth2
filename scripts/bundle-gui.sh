#!/usr/bin/env bash
# Wrap the portable window in a macOS .app bundle.
# Usage: scripts/bundle-gui.sh [release|debug]
set -euo pipefail

cd "$(dirname "$0")/.."
CONFIG="${1:-release}"
APP="build/BerthGui.app"
BIN="core/target/$CONFIG/berth-gui"

if [[ ! -x "$BIN" ]]; then
  echo "bundle-gui: missing $BIN (run: make gui)" >&2
  exit 1
fi

# A shallow CI checkout has no tags, so a failed describe must not abort under
# `set -e`; callers can pass VERSION/BUILD explicitly.
VERSION="${VERSION:-$(git describe --tags --abbrev=0 2>/dev/null || echo 0.0.0)}"
VERSION="${VERSION#v}"
BUILD="${BUILD:-$(git rev-list --count HEAD 2>/dev/null || echo 1)}"

echo "==> assembling $APP ($VERSION build $BUILD)"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/berth-gui"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Berth Portable</string>
  <key>CFBundleDisplayName</key><string>Berth Portable</string>
  <key>CFBundleIdentifier</key><string>dev.berth.gui</string>
  <key>CFBundleExecutable</key><string>berth-gui</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${BUILD}</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST

echo "==> ad-hoc signing"
codesign --force --sign - "$APP"
echo "ok: $APP ($VERSION)"
