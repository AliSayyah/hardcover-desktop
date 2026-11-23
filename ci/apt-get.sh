#!/usr/bin/env bash
# apt-get for CI, with its network waits bounded. The Ubuntu mirrors now and
# then stall or serve a trickle, and apt's own timeouts only notice a
# connection that goes silent: on 2026-11-23 a two-package install sat in
# `apt-get update` for 22 minutes, and would have held the job for GitHub's
# six hours. Here each network phase (refreshing the package lists,
# downloading the packages) has a time limit and is tried again, on a new
# connection, when it runs out or fails. Installing comes after, offline and
# under no limit: killing dpkg halfway would leave packages half-configured.
#
#   ci/apt-get.sh install [--update] [--download-only] PACKAGE...
#
# install first tries the package lists already there (the runner image's, or
# the CI cache's), and refreshes them only if they don't name the packages or
# the mirror no longer has the files they name. With --update it refreshes
# them first, so the packages are checked against freshly signed lists.
# --download-only stops once the packages are downloaded.
#
# It runs as root, or through sudo. ci/apt-get-test.sh replaces apt-get
# (APT_GET), sudo (SUDO) and the limits (APT_ATTEMPTS, APT_UPDATE_LIMIT and
# APT_DOWNLOAD_LIMIT, in seconds, and APT_RETRY_DELAY).
set -euo pipefail

apt_get=${APT_GET:-apt-get}
if [[ -v SUDO ]]; then
  sudo=$SUDO
elif [[ $(id -u) != 0 ]]; then
  sudo=sudo
else
  sudo=
fi
attempts=${APT_ATTEMPTS:-3}
# A healthy refresh takes seconds and the build dependencies' download
# (about 90 MB) well under a minute; a slow mirror has served them at a few
# hundred kB/s.
update_limit=${APT_UPDATE_LIMIT:-240}
download_limit=${APT_DOWNLOAD_LIMIT:-600}
retry_delay=${APT_RETRY_DELAY:-10}
# A connection silent for 30 s (not apt's default 120) is dropped, and a file
# that fails is fetched again by apt itself before the attempt fails.
options=(-o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30)

die() {
  echo "apt-get.sh: $*" >&2
  exit 2
}

warn() {
  if [[ ${GITHUB_ACTIONS:-} == true ]]; then
    echo "::warning::$*"
  else
    echo "apt-get.sh: $*" >&2
  fi
}

# bounded TRIES LIMIT WHAT ARG...: runs apt-get ARG... up to TRIES times,
# each within LIMIT seconds.
bounded() {
  local tries=$1 limit=$2 what=$3 attempt status
  shift 3
  for ((attempt = 1; ; attempt++)); do
    status=0
    $sudo timeout --kill-after=15 "$limit" "$apt_get" "${options[@]}" "$@" || status=$?
    ((status != 0)) || return 0
    ((tries > 1)) || return "$status"
    # 124: the limit ran out; 137: it ignored the TERM and was killed.
    if ((status == 124 || status == 137)); then
      warn "$what took over $limit s (attempt $attempt of $tries)"
    else
      warn "$what failed (attempt $attempt of $tries)"
    fi
    ((attempt < tries)) || return "$status"
    sleep "$retry_delay"
  done
}

refresh() {
  bounded "$attempts" "$update_limit" "refreshing the package lists" update -qq
}

# download TRIES PACKAGE...
download() {
  local tries=$1
  shift
  bounded "$tries" "$download_limit" "downloading $*" \
    install -y -qq --no-install-recommends --download-only "$@"
}

[[ ${1:-} == install ]] || die "usage: ci/apt-get.sh install [--update] [--download-only] PACKAGE..."
shift
update=0 download_only=0
while [[ ${1:-} == --* ]]; do
  case $1 in
    --update) update=1 ;;
    --download-only) download_only=1 ;;
    *) die "unknown option $1" ;;
  esac
  shift
done
(($# > 0)) || die "no packages to install"

if ((update)); then
  refresh
  download "$attempts" "$@"
elif ! download 1 "$@" 2>/dev/null; then
  echo "apt-get.sh: the package lists here don't have $*; refreshing them" >&2
  refresh
  download "$attempts" "$@"
fi
((download_only)) ||
  $sudo env DEBIAN_FRONTEND=noninteractive "$apt_get" "${options[@]}" install -y -qq --no-install-recommends "$@"
