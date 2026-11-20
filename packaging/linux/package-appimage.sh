#!/usr/bin/env bash
# Build on the same distribution as the release binary. linuxdeploy resolves
# the libraries to bundle from the build system.
set -euo pipefail

archive=$(realpath "$1")
name=$(basename "$archive" .tar.gz)
out=$(realpath "$(dirname "$archive")")
[[ $name == *-x86_64-linux && $(uname -m) == x86_64 ]]
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
