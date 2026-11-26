#!/usr/bin/env bash
# Regenerate the packaged icons from packaging/berth.svg: Berth.icns (macOS),
# berth-icon.png (portable window), berth.ico (Windows executable).
set -euo pipefail

cd "$(dirname "$0")/.."
SVG="packaging/berth.svg"
OUT="packaging/Berth.icns"
PNG="packaging/berth-icon.png"
ICO="packaging/berth.ico"

have() { command -v "$1" >/dev/null 2>&1; }

# Each format needs its own tool, so a host missing one still writes the rest.
if ! have rsvg-convert; then
  echo "icon: rsvg-convert missing (brew install librsvg)" >&2
  exit 1
fi

rsvg-convert -w 256 -h 256 "$SVG" -o "$PNG"
echo "ok: $PNG"

if have magick; then
  magick "$SVG" -background none -define icon:auto-resize=256,128,64,48,32,16 "$ICO"
  echo "ok: $ICO"
elif have convert; then
  convert "$SVG" -background none -define icon:auto-resize=256,128,64,48,32,16 "$ICO"
  echo "ok: $ICO"
else
  echo "icon: ImageMagick missing, could not write $ICO (brew install imagemagick)" >&2
  exit 1
fi

if have iconutil; then
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  iconset="$work/Berth.iconset"
  mkdir -p "$iconset"

  # Every representation iconutil expects, at the size its @2x suffix implies.
  for spec in \
    16:icon_16x16 32:icon_16x16@2x \
    32:icon_32x32 64:icon_32x32@2x \
    128:icon_128x128 256:icon_128x128@2x \
    256:icon_256x256 512:icon_256x256@2x \
    512:icon_512x512 1024:icon_512x512@2x; do
    pixels="${spec%%:*}"
    name="${spec##*:}"
    rsvg-convert -w "$pixels" -h "$pixels" "$SVG" -o "$iconset/$name.png"
  done

  iconutil -c icns "$iconset" -o "$OUT"
  echo "ok: $OUT"
else
  echo "icon: iconutil missing (macOS only), left $OUT unchanged" >&2
fi
