#!/usr/bin/env bash
# Builds the release tarball from an already-built binary.
#
#   packaging/linux/package.sh <version> <binary> <out-dir>
#
# The tarball unpacks to hardcover-desktop-<version>-<arch>-linux/ with an
# FHS-style tree, so it can be installed into a prefix with
#   tar -xzf <tarball> -C ~/.local --strip-components=1
set -euo pipefail

version=$1 binary=$2 out=$3
app_id=io.github.alisayyah.HardcoverDesktop
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
arch=$(uname -m)
name="hardcover-desktop-${version}-${arch}-linux"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
dir="$stage/$name"

install -Dm755 "$binary" "$dir/bin/hardcover-desktop"
install -Dm644 "$here/$app_id.desktop" "$dir/share/applications/$app_id.desktop"
install -Dm644 "$here/$app_id.metainfo.xml" "$dir/share/metainfo/$app_id.metainfo.xml"
install -Dm644 "$root/assets/icon.svg" "$dir/share/icons/hicolor/scalable/apps/$app_id.svg"
install -Dm644 "$root/LICENSE" "$dir/share/licenses/hardcover-desktop/LICENSE"
# The bundled fonts (SIL Open Font License); each licence must travel with
# its font. Every directory under assets/fonts holds one family and its
# LICENSE.txt, installed as <directory>-LICENSE.txt (inter-LICENSE.txt, ...).
for font_dir in "$root"/assets/fonts/*/; do
    font=$(basename "$font_dir")
    if [[ ! -f "$font_dir/LICENSE.txt" ]]; then
        echo "error: $font_dir has no LICENSE.txt" >&2
        exit 1
    fi
    install -Dm644 "$font_dir/LICENSE.txt" "$dir/share/licenses/hardcover-desktop/$font-LICENSE.txt"
done

mkdir -p "$out"
tar -C "$stage" --owner=0 --group=0 --numeric-owner -czf "$out/$name.tar.gz" "$name"
(cd "$out" && sha256sum "$name.tar.gz" >"$name.tar.gz.sha256")
echo "$out/$name.tar.gz"
