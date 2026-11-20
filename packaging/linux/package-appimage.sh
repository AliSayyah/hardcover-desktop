#!/usr/bin/env bash
# Builds the AppImage from the release tarball, next to it.
#
#   packaging/linux/package-appimage.sh <out-dir>/hardcover-desktop-<version>-x86_64-linux.tar.gz
#
# Run it on the distribution the binary was built on (Ubuntu 22.04 for a
# release): linuxdeploy bundles the libraries the binary links from this
# system, so they must match the glibc it was built against.
set -euo pipefail

[[ $# == 1 ]] || { echo "usage: $0 <release tarball>" >&2; exit 2; }
archive=$(realpath "$1")
name=$(basename "$archive" .tar.gz)
out=$(realpath "$(dirname "$archive")")
if [[ $name != *-x86_64-linux || $(uname -m) != x86_64 ]]; then
    echo "error: needs an x86_64 release tarball, on x86_64: $1" >&2
    exit 1
fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"
mkdir -p AppDir/usr
tar -xzf "$archive" --strip-components=1 -C AppDir/usr
# linuxdeploy finds the other package notices itself. For these libraries on
# Ubuntu 22.04 its dpkg lookup misses the /lib to /usr/lib aliases.
if command -v dpkg-query >/dev/null; then
    for package in libdbus-1-3 liblzma5 libcap2; do
        install -Dm644 "/usr/share/doc/$package/copyright" "AppDir/usr/share/doc/$package/copyright"
    done
fi

# Both inputs are immutable releases. Pass the runtime explicitly so the
# packaging tool never fetches the moving "continuous" runtime.
curl --proto '=https' -fsSL --retry 2 -o linuxdeploy \
    https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage
curl --proto '=https' -fsSL --retry 2 -o runtime \
    https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64
sha256sum -c <<'DIGESTS'
c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d  linuxdeploy
2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d  runtime
DIGESTS
chmod +x linuxdeploy
export LDAI_RUNTIME_FILE="$work/runtime" LDAI_OUTPUT="$work/$name.AppImage"
./linuxdeploy --appimage-extract-and-run --appdir AppDir --output appimage
chmod +x "$name.AppImage"
./"$name.AppImage" --appimage-extract-and-run --version
mv -- "$name.AppImage" "$out/"
