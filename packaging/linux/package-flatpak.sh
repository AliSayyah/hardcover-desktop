#!/usr/bin/env bash
# Builds a single-file Flatpak bundle from the release tarball, next to it.
#
#   packaging/linux/package-flatpak.sh <out-dir>/hardcover-desktop-<version>-x86_64-linux.tar.gz
#
# The tarball's tree becomes /app as it is, with flatpak.metadata (runtime and
# sandbox permissions). Only the flatpak CLI is needed: nothing is compiled,
# and the Freedesktop runtime is installed from Flathub with the bundle.
set -euo pipefail

[[ $# == 1 ]] || { echo "usage: $0 <release tarball>" >&2; exit 2; }
archive=$(realpath "$1")
name=$(basename "$archive" .tar.gz)
out=$(dirname "$archive")
if [[ $name != *-x86_64-linux ]]; then
    echo "error: needs an x86_64 release tarball: $1" >&2
    exit 1
fi
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"
mkdir -p build/files
cp "$here/flatpak.metadata" build/metadata
tar -xzf "$archive" --strip-components=1 -C build/files
flatpak build-finish build
app_id=io.github.alisayyah.HardcoverDesktop
flatpak build-export --arch=x86_64 repo build stable
flatpak build-bundle --arch=x86_64 --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
    repo "$name.flatpak" "$app_id" stable
mv -- "$name.flatpak" "$out/"
