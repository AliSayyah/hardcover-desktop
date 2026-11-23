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
# (about 90 MB) well under a minute, but a slow mirror has taken minutes for
# either and still finished: on 2026-11-23 one refresh ran past 4 minutes
# three times in a row, while the next job's second try took under 2. So
# the limits are generous, and only a wait far past them is cut short.
update_limit=${APT_UPDATE_LIMIT:-480}
download_limit=${APT_DOWNLOAD_LIMIT:-900}
retry_delay=${APT_RETRY_DELAY:-10}
# The time goes on connections that go silent, not on slow ones: on
# 2026-11-23 a refresh fetched its 17.5 MB list in seconds, then spent 5
# minutes on 920 kB because connections to archive.ubuntu.com went silent
# again and again, each costing a timeout before apt asked again. A mirror
# that works answers within a second, so a connection silent for 10 s (apt's
# default is 120) is dropped, and apt fetches the file again itself, up to 3
# times, before the attempt fails. A new attempt keeps the lists already
# fetched.
options=(-o Acquire::Retries=3 -o Acquire::http::Timeout=10 -o Acquire::https::Timeout=10)

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
  # -q, not -qq: a line per list fetched, so a slow one shows which.
  bounded "$attempts" "$update_limit" "refreshing the package lists" update -q
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
