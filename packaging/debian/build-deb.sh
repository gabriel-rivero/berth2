#!/usr/bin/env bash
# Build a Debian package from prebuilt Linux binaries.
# Usage: packaging/debian/build-deb.sh VERSION [BIN_DIR] [OUT_DIR]
set -euo pipefail

version="${1:?usage: build-deb.sh VERSION [BIN_DIR] [OUT_DIR]}"
bin_dir="${2:-core/target/release}"
out_dir="${3:-dist}"
arch="${ARCH:-amd64}"
package="berth_${version}_${arch}"

root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT
pkg="$root/$package"
mkdir -p "$pkg/DEBIAN" "$pkg/usr/bin" "$pkg/usr/share/applications" \
  "$pkg/usr/share/icons/hicolor/scalable/apps" "$pkg/usr/share/doc/berth"

install -m 0755 "$bin_dir/berth" "$bin_dir/berth-gui" "$pkg/usr/bin/"
install -m 0644 packaging/berth.desktop "$pkg/usr/share/applications/berth.desktop"
install -m 0644 packaging/berth.svg \
  "$pkg/usr/share/icons/hicolor/scalable/apps/berth.svg"
install -m 0644 README.md LICENSE "$pkg/usr/share/doc/berth/"

cat > "$pkg/DEBIAN/control" <<CONTROL
Package: berth
Version: $version
Section: x11
Priority: optional
Architecture: $arch
Depends: libc6, libgcc-s1
Maintainer: Gabriel Rivero <gabriel.rivero.sa@gmail.com>
Homepage: https://github.com/gabriel-rivero/berth2
Description: Terminal workspace for coding agents
 Berth is a native terminal workspace for running coding agents side by side.
 It keeps a typed agent lifecycle, a workspace/tab layout and session restore,
 and exposes a local socket protocol for scripts and hooks.
CONTROL

mkdir -p "$out_dir"
dpkg-deb --build --root-owner-group "$pkg" "$out_dir/$package.deb"
echo "ok: $out_dir/$package.deb"
