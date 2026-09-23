#!/usr/bin/env bash
# Points packaging/arch/PKGBUILD at a published release: sets pkgver, resets
# pkgrel, pins the checksums of the release .deb and LICENSE, regenerates
# .SRCINFO. Run it after the release workflow has uploaded the bundles.
set -euo pipefail

version="${1:-}"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: scripts/update-pkgbuild.sh <major.minor.patch>" >&2
  exit 1
fi

cd "$(dirname "$0")/../packaging/arch"

base="https://github.com/NiklasRhf/yaydl"
deb_url="$base/releases/download/yaydl-v$version/yaydl_${version}_amd64.deb"
license_url="https://raw.githubusercontent.com/NiklasRhf/yaydl/yaydl-v$version/LICENSE"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/yaydl.deb" "$deb_url"
curl -fsSL -o "$tmp/LICENSE" "$license_url"
deb_sha=$(sha256sum "$tmp/yaydl.deb" | cut -d' ' -f1)
license_sha=$(sha256sum "$tmp/LICENSE" | cut -d' ' -f1)

sed -i \
  -e "s/^pkgver=.*/pkgver=$version/" \
  -e "s/^pkgrel=.*/pkgrel=1/" \
  -e "/^sha256sums=(/,/)/c\\sha256sums=('$deb_sha'\\n            '$license_sha')" \
  PKGBUILD

makepkg --printsrcinfo > .SRCINFO
grep -q "pkgver = $version" .SRCINFO || { echo ".SRCINFO was not updated" >&2; exit 1; }
echo "PKGBUILD and .SRCINFO now point at yaydl $version"
