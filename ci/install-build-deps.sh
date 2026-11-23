#!/usr/bin/env bash
# Installs what building the workspace needs on Ubuntu 22.04 or newer: a C
# toolchain and the system libraries the -sys crates link against. Wayland,
# Vulkan and OpenGL are loaded at run time, so they are not needed to build.
set -euo pipefail

packages=(
  build-essential
  ca-certificates
  curl
  git
  pkg-config
  libdbus-1-dev
  libfontconfig-dev
  libxcb1-dev
  libxkbcommon-x11-dev
)

here=$(dirname "${BASH_SOURCE[0]}")
# CI restores the package lists along with the packages from its cache and
# sets APT_LISTS_CACHED=1, so a slow mirror can't stall the job; apt-get.sh
# refreshes them if they no longer work. Otherwise the lists are refreshed
# first.
if [[ ${APT_LISTS_CACHED:-} == 1 ]]; then
  "$here/apt-get.sh" install "${packages[@]}"
else
  "$here/apt-get.sh" install --update "${packages[@]}"
fi
