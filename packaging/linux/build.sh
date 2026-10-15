#!/bin/sh
# Builds the Linux downloads from target/release/DeskChalk:
#   dist/DeskChalk_Setup_Linux_v<version>.deb      (installs DeskChalk for Debian, Ubuntu, Mint, ...)
#   dist/DeskChalk_Portable_Linux_v<version>.tar.gz
# Usage: packaging/linux/build.sh 0.3.0 [package-version]   (needs dpkg-deb; rsvg-convert adds PNG icons)
# The package version defaults to the version. Test builds pass e.g. 0.3.0~ci12: each one then
# counts as an update to the one before, and as older than the real 0.3.0.
set -eu

version="$1"
pkgversion="${2:-$1}"
root=$(cd "$(dirname "$0")/../.." && pwd)
here="$root/packaging/linux"
bin="$root/target/release/DeskChalk"
out="$root/dist"
mkdir -p "$out"

pkg=$(mktemp -d)
tmp=$(mktemp -d)
trap 'rm -rf "$pkg" "$tmp"' EXIT

install -D -m 755 "$bin" "$pkg/usr/bin/deskchalk"
install -D -m 644 "$here/deskchalk.desktop" "$pkg/usr/share/applications/deskchalk.desktop"
install -D -m 644 "$root/assets/deskchalk.svg" "$pkg/usr/share/icons/hicolor/scalable/apps/deskchalk.svg"
if command -v rsvg-convert >/dev/null 2>&1; then
  for s in 16 22 24 32 48 64 128 256; do
    mkdir -p "$pkg/usr/share/icons/hicolor/${s}x${s}/apps"
    rsvg-convert -w "$s" -h "$s" "$root/assets/deskchalk.svg" -o "$pkg/usr/share/icons/hicolor/${s}x${s}/apps/deskchalk.png"
  done
fi
install -D -m 644 "$root/LICENSE.txt" "$pkg/usr/share/doc/deskchalk/copyright"

size=$(du -sk "$pkg/usr" | cut -f1)
mkdir -p "$pkg/DEBIAN"
sed -e "s/@VERSION@/$pkgversion/" -e "s/@SIZE@/$size/" "$here/control" > "$pkg/DEBIAN/control"
dpkg-deb --root-owner-group --build "$pkg" "$out/DeskChalk_Setup_Linux_v$version.deb"

mkdir "$tmp/DeskChalk"
install -m 755 "$bin" "$tmp/DeskChalk/deskchalk"
cp "$root/LICENSE.txt" "$tmp/DeskChalk/"
tar -C "$tmp" -czf "$out/DeskChalk_Portable_Linux_v$version.tar.gz" DeskChalk

ls -l "$out"
