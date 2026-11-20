#!/usr/bin/env bash
# Install the latest stable release, or a local release tarball and its .minisig.
set -euo pipefail

die() { echo "install: $*" >&2; exit 1; }
[[ $# -le 1 ]] || die "usage: bash install.sh [hardcover-desktop-<version>-x86_64-linux.tar.gz]"
[[ $(uname -sm) == 'Linux x86_64' ]] || die "this release requires x86_64 Linux"

releases=https://github.com/AliSayyah/hardcover-desktop/releases
public_key=RWQmH1HvOAVP+8piXBk3ZC+nqzBVFZFcynEg8NUsgQDCSYYAkrSkPRXW
work=$(mktemp -d)
stage=
cleanup() {
    rm -rf -- "$work"
    if [[ -n $stage ]]; then rm -rf -- "$stage"; fi
}
trap cleanup EXIT
fetch() { curl --proto '=https' --proto-redir '=https' -fsSL --retry 2 "$@"; }

if [[ $# == 1 ]]; then
    name=$(basename -- "$1")
    cp -- "$1" "$1.minisig" "$work/"
else
    url=$(fetch -o /dev/null -w '%{url_effective}' "$releases/latest")
    tag=${url##*/}
    [[ $url == "$releases/tag/$tag" && $tag =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
        die "could not identify the latest stable release"
    name="hardcover-desktop-${tag#v}-x86_64-linux.tar.gz"
    fetch "$releases/download/$tag/$name" -o "$work/$name"
    fetch "$releases/download/$tag/$name.minisig" -o "$work/$name.minisig"
fi
[[ $name =~ ^hardcover-desktop-[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?-x86_64-linux\.tar\.gz$ ]] ||
    die "unexpected release filename: $name"

# Use the system verifier if available. Otherwise use the same pinned static
# minisign release as the signing workflow, only for this installation.
verifier=$(command -v minisign || true)
if [[ -z $verifier ]]; then
    fetch https://github.com/jedisct1/minisign/releases/download/0.12/minisign-0.12-linux.tar.gz -o "$work/verifier.tar.gz"
    (cd "$work" && echo '9a599b48ba6eb7b1e80f12f36b94ceca7c00b7a5173c95c3efc88d9822957e73  verifier.tar.gz' | sha256sum -c -)
    tar --warning=no-unknown-keyword -xzf "$work/verifier.tar.gz" -C "$work" minisign-linux/x86_64/minisign
    verifier="$work/minisign-linux/x86_64/minisign"
fi
"$verifier" -Vm "$work/$name" -P "$public_key"
# The updater also binds the authenticated filename to the requested release.
signed_name=$(sed -n '3s/^trusted comment: //p' "$work/$name.minisig" | tr '\t' '\n' | sed -n 's/^file://p')
[[ $signed_name == "$name" ]] || die "the signature names a different release"

prefix="$HOME/.local"
mkdir -p -- "$prefix"
stage=$(mktemp -d "$prefix/.hardcover-install.XXXXXXXX")
tar -xzf "$work/$name" --no-same-owner -C "$stage"
payload="$stage/${name%.tar.gz}"
[[ -x $payload/bin/hardcover-desktop ]] || die "the archive is missing the app"
# Serialize installers and replace each file by rename, with the executable last.
exec 9<"$prefix"
flock 9
cd "$payload"
find share -type f -print0 > "$stage/share-files"
while IFS= read -r -d '' rel; do
    mkdir -p -- "$prefix/${rel%/*}"
    mv -fT -- "$rel" "$prefix/$rel"
done < "$stage/share-files"
mkdir -p -- "$prefix/bin"
mv -fT -- bin/hardcover-desktop "$prefix/bin/hardcover-desktop"
echo "Installed Hardcover Desktop. Run $prefix/bin/hardcover-desktop or open it from your app launcher."
