#!/usr/bin/env bash
# Tests ci/live-api-report.sh offline, with a gh that serves a list of open
# issues and logs what it is asked to do: the verdict (its exit code), the
# issues it opens, comments on and closes, and their words.
#
#   ci/live-api-report-test.sh      (`just live-api-report-test`)
set -uo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
calls=$tmp/calls
bodies=$tmp/bodies
issues=$tmp/issues

# Logs each call as one word per action (list, label, create:<title>,
# comment:<n>, close:<n>) in $CALLS and the text it sends in $BODIES.
# `issue list` prints $ISSUES (number TAB title per line) whatever it is asked,
# as the search is fuzzy; anything unscripted fails.
cat >"$tmp/gh" <<'EOF'
#!/usr/bin/env bash
set -uo pipefail
fail() { echo "fake gh: $*" >&2; exit 1; }
sub="$1 $2"
shift 2
declare -A flag=()
pos=()
while (($#)); do
  case $1 in
    --force) flag[--force]=1; shift ;;
    --*) flag[$1]=$2; shift 2 ;;
    *) pos+=("$1"); shift ;;
  esac
done
case $sub in
  "issue list")
    [[ ${flag[--label]-} == live-api && ${flag[--state]-} == open && ${flag[--json]-} == number,title ]] ||
      fail "issue list without --label live-api --state open --json number,title"
    [[ ${flag[--search]-} == 'in:title "'*'"' && -n ${flag[--template]-} ]] || fail "issue list: bad --search or no --template"
    echo list >>"$CALLS"
    cat "$ISSUES"
    ;;
  "label create")
    [[ ${pos[0]-} == live-api && ${flag[--force]-} == 1 ]] || fail "label create: ${pos[*]}"
    echo label >>"$CALLS"
    ;;
  "issue create")
    [[ ${flag[--label]-} == live-api && ${flag[--assignee]-} == AliSayyah && -n ${flag[--title]-} ]] ||
      fail "issue create without --label live-api --assignee AliSayyah --title"
    echo "create:${flag[--title]}" >>"$CALLS"
    printf '%s\n' "${flag[--body]}" >>"$BODIES"
    echo "https://github.com/o/r/issues/99"
    ;;
  "issue comment")
    [[ -n ${pos[0]-} && -n ${flag[--body]-} ]] || fail "issue comment: no number or body"
    echo "comment:${pos[0]}" >>"$CALLS"
    printf '%s\n' "${flag[--body]}" >>"$BODIES"
    ;;
  "issue close")
    [[ -n ${pos[0]-} && -n ${flag[--comment]-} ]] || fail "issue close: no number or comment"
    echo "close:${pos[0]}" >>"$CALLS"
    printf '%s\n' "${flag[--comment]}" >>"$BODIES"
    ;;
  *) fail "unscripted gh $sub" ;;
esac
EOF
chmod +x "$tmp/gh"
export GH=$tmp/gh CALLS=$calls BODIES=$bodies ISSUES=$issues
export GITHUB_SERVER_URL=https://github.com GITHUB_REPOSITORY=o/r GITHUB_RUN_ID=42
unset GITHUB_STEP_SUMMARY GH_TOKEN GH_REPO

failed="Live API check failed"
drift="Live API: the schema changed"
# Open issues: a near miss on each title, which must never be touched.
near_misses="8	Live API check failed (old)
9	Live API: the schema changed?"

failures=0
# case_ NAME EXPECTED-STATUS EXPECTED-CALLS OPEN-ISSUES WORDS API_CHECK LIVE_SUITE
# WORDS must all appear in what gh was sent (a |-separated list, or empty).
case_() {
  local name=$1 want_status=$2 want_calls=$3 open=$4 words=$5 status got word
  shift 5
  : >"$calls"
  : >"$bodies"
  printf '%s\n%s\n' "$open" "$near_misses" >"$issues"
  "$here/live-api-report.sh" "$@" >"$tmp/out" 2>&1
  status=$?
  got=$(paste -sd' ' "$calls")
  if [[ $status != "$want_status" || $got != "$want_calls" ]]; then
    echo "FAIL $name: status $status (want $want_status), calls '$got' (want '$want_calls')"
    sed 's/^/  /' "$tmp/out"
    failures=$((failures + 1))
    return
  fi
  if [[ -n $words ]]; then
    IFS='|' read -ra list <<<"$words"
    for word in "${list[@]}"; do
      if ! grep -qF -- "$word" "$bodies"; then
        echo "FAIL $name: gh was never sent '$word'"
        sed 's/^/  /' "$bodies"
        failures=$((failures + 1))
        return
      fi
    done
  fi
  echo "ok   $name"
}

run_url=https://github.com/o/r/actions/runs/42

case_ "all pass, no open issue" 0 "list list" \
  "" "" 0 0
case_ "all pass closes the open failure and drift issues" 0 "list close:7 list close:5" \
  "7	$failed
5	$drift" "Passed again: $run_url|matches the live API again" 0 0
case_ "a breakage with no issue opens one" 1 "list label create:$failed list" \
  "" "broken against the live|$run_url|\`live_suite\`: **broken**" 0 1
case_ "a breakage with an issue comments on it" 1 "list comment:7" \
  "7	$failed" "broken against the live|(exit 1)" 1 0
case_ "a broken schema check doesn't touch the drift issue" 1 "list comment:7" \
  "7	$failed
5	$drift" "" 1 0
case_ "a rejected token says to replace the secret" 1 "list label create:$failed list" \
  "" "replace the \`HARDCOVER_TEST_TOKEN\` secret|(exit 4)" 0 4
case_ "broken wins over a rejected token" 1 "list label create:$failed" \
  "" "broken against the live" 2 1
case_ "a rejected token wins over api_check's setup error" 1 "list label create:$failed" \
  "" "HARDCOVER_TEST_TOKEN" 2 4
case_ "drift alone passes and opens the drift issue" 0 "list list label create:$drift" \
  "" "just schema-update|$run_url" 3 0
case_ "drift alone comments on the drift issue, closes the failure issue" 0 "list close:7 list comment:5" \
  "7	$failed
5	$drift" "Passed again" 3 0
case_ "an inconclusive live suite is a failure, worded so" 1 "list label create:$failed list" \
  "" "Inconclusive|(exit 3)" 0 3
case_ "a setup error is a failure, worded as inconclusive" 1 "list label create:$failed" \
  "" "Inconclusive|could not run" 2 0
case_ "steps that never ran are a failure" 1 "list label create:$failed" \
  "" "did not run" "" ""
case_ "a panic is a failure" 1 "list label create:$failed list" \
  "" "failed unexpectedly|(exit 101)" 0 101
case_ "an unexpected code from api_check is a failure" 1 "list label create:$failed" \
  "" "failed unexpectedly" 101 0
case_ "a missing argument" 2 "" "" "" 0

if ((failures > 0)); then
  echo "$failures failed"
  exit 1
fi
