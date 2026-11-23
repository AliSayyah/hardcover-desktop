#!/usr/bin/env bash
# Tests ci/apt-get.sh offline, with an apt-get that plays a script: that it
# uses the lists it finds, refreshes them when they fail, gives up on a phase
# that hangs and tries it again, and never puts the install itself under a
# limit.
#
#   ci/apt-get-test.sh      (`just apt-test`)
set -uo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
calls=$tmp/calls
script=$tmp/script

# Each call logs what it is (update, download or install) and takes the next
# line of $script: ok, fail, or hang (sleeps far past every limit), or
# "slow N" (sleeps N seconds, then succeeds).
cat >"$tmp/apt-get" <<'EOF'
#!/usr/bin/env bash
case " $* " in
  *" update "*) what=update ;;
  *" --download-only "*) what=download ;;
  *) what=install ;;
esac
echo "$what" >>"$CALLS"
line=$(head -n1 "$SCRIPT")
sed -i 1d "$SCRIPT"
case $line in
  ok) exit 0 ;;
  fail) exit 100 ;;
  hang) exec sleep 60 ;;
  slow\ *) exec sleep "${line#slow }" ;;
  *) echo "unscripted apt-get $what" >&2; exit 1 ;;
esac
EOF
chmod +x "$tmp/apt-get"
export APT_GET=$tmp/apt-get SUDO='' CALLS=$calls SCRIPT=$script
export APT_ATTEMPTS=3 APT_UPDATE_LIMIT=1 APT_DOWNLOAD_LIMIT=1 APT_RETRY_DELAY=0
unset GITHUB_ACTIONS

failures=0
# case NAME EXPECTED-STATUS EXPECTED-CALLS SCRIPT ARG...
case_() {
  local name=$1 want_status=$2 want_calls=$3 answers=$4 status start elapsed got
  shift 4
  : >"$calls"
  tr ' ' '\n' <<<"$answers" | sed 's/_/ /' >"$script"
  start=$SECONDS
  "$here/apt-get.sh" "$@" >/dev/null 2>"$tmp/err"
  status=$?
  elapsed=$((SECONDS - start))
  got=$(paste -sd' ' "$calls")
  if [[ $status != "$want_status" || $got != "$want_calls" ]]; then
    echo "FAIL $name: status $status (want $want_status), calls '$got' (want '$want_calls')"
    sed 's/^/  /' "$tmp/err"
    failures=$((failures + 1))
  elif ((elapsed > 30)); then
    echo "FAIL $name: took ${elapsed}s, so a hang wasn't cut short"
    failures=$((failures + 1))
  else
    echo "ok   $name"
  fi
}

case_ "the lists here have the packages" 0 "download install" \
  "ok ok" install pkg
case_ "stale lists are refreshed" 0 "download update download install" \
  "fail ok ok ok" install pkg
case_ "--update refreshes first" 0 "update download install" \
  "ok ok ok" install --update pkg
case_ "--download-only installs nothing" 0 "download" \
  "ok" install --download-only pkg
case_ "a refresh that hangs is cut short and tried again" 0 "update update download install" \
  "hang ok ok ok" install --update pkg
case_ "a failed download is tried again" 0 "update download download install" \
  "ok fail ok ok" install --update pkg
case_ "a download that always hangs gives up, never installing" 124 "update download download download" \
  "ok hang hang hang" install --update pkg
case_ "a refresh that always fails gives up" 100 "download update update update" \
  "fail fail fail fail" install pkg
case_ "the install has no limit" 0 "download install" \
  "ok slow_2" install pkg
case_ "no packages" 2 "" "" install
case_ "an unknown option" 2 "" "" install --upgrade pkg

if ((failures > 0)); then
  echo "$failures failed"
  exit 1
fi
