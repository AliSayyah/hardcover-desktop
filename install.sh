#!/usr/bin/env bash
# Installs Hardcover Desktop under ~/.local, where it can update itself.
#
#   curl -fsSL https://github.com/AliSayyah/hardcover-desktop/releases/latest/download/install.sh | bash
#   bash install.sh                 the latest stable release, from GitHub
#   bash install.sh <tarball>       a release tarball, with its .minisig beside it
#
# The tarball must be signed by the release key (packaging/minisign.pub; a
# test keeps the copy below in step), and the signature must name the file,
# as the in-app updater requires.
set -euo pipefail

# Everything runs from main, called on the last line, so a download through a
# pipe that breaks off halfway runs nothing.
main() {
    die() { echo "install: $*" >&2; exit 1; }
    [[ $# -le 1 ]] || die "usage: bash install.sh [hardcover-desktop-<version>-x86_64-linux.tar.gz]"
    [[ $(uname -sm) == 'Linux x86_64' ]] || die "this release requires x86_64 Linux"

    releases=https://github.com/AliSayyah/hardcover-desktop/releases
    public_key=RWQmH1HvOAVP+8piXBk3ZC+nqzBVFZFcynEg8NUsgQDCSYYAkrSkPRXW
    work=$(mktemp -d)
    stage=
    copies=() # files waiting beside their destinations, and where they go
    dests=()
    cleanup() {
        rm -rf -- "$work"
        if [[ -n $stage ]]; then rm -rf -- "$stage"; fi
        # The ones not renamed yet, if it stopped halfway.
        rm -f -- "${copies[@]}"
    }
    trap cleanup EXIT
    fetch() { curl --proto '=https' --proto-redir '=https' -fsSL --retry 2 "$@"; }

    if [[ $# == 1 ]]; then
        name=$(basename -- "$1")
        [[ -f $1 ]] || die "no such file: $1"
        [[ -f $1.minisig ]] || die "put the signature, $name.minisig, next to the tarball"
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
        (cd "$work" && echo '9a599b48ba6eb7b1e80f12f36b94ceca7c00b7a5173c95c3efc88d9822957e73  verifier.tar.gz' | sha256sum --quiet -c -)
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
    # Serialize with other installers and the app's own updater, which takes
    # the same lock. Each file is copied next to its destination first, so the
    # rename that replaces it stays on one filesystem even when bin/ or share/
    # links to another; then all are renamed, the executable last. Copies a
    # crashed run left (the updater's name prefix, made only under this lock)
    # are removed first.
    exec 9<"$prefix"
    flock 9
    cd "$payload"
    temp=.hardcover-desktop-update-
    declare -A swept=()
    find share -type f -print0 > "$stage/files"
    printf '%s\0' bin/hardcover-desktop >> "$stage/files"
    while IFS= read -r -d '' rel; do
        dir="$prefix/$(dirname -- "$rel")"
        mkdir -p -- "$dir"
        if [[ -z ${swept[$dir]:-} ]]; then
            find -H "$dir" -maxdepth 1 -type f -name "$temp*" -delete
            swept[$dir]=1
        fi
        copy=$(mktemp "$dir/${temp}XXXXXXXX")
        copies+=("$copy")
        dests+=("$prefix/$rel")
        cp -- "$rel" "$copy"
        chmod --reference="$rel" -- "$copy"
    done < "$stage/files"
    sync -- "${copies[@]}"
    for i in "${!copies[@]}"; do
        mv -fT -- "${copies[i]}" "${dests[i]}"
    done
    sync -- "${!swept[@]}"
    echo "Installed Hardcover Desktop. Open it from your app launcher, or run $prefix/bin/hardcover-desktop."
    # The desktop entry runs the binary by name, so the session needs it on PATH.
    # Some distributions add ~/.local/bin only if it existed when you logged in.
    case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "$prefix/bin is not on your PATH yet: if the launcher can't start the app, log out and back in." ;;
    esac
}

main "$@"
