#!/usr/bin/env bash
# Export the existing release payload with the Freedesktop runtime. Packaging
# a prebuilt app only needs the Flatpak CLI. The runtime is installed at use.
set -euo pipefail

archive=$(realpath "$1")
name=$(basename "$archive" .tar.gz)
out=$(dirname "$archive")
[[ $name == *-x86_64-linux ]]
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
