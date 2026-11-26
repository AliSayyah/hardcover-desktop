#!/usr/bin/env bash
# The verdict of the weekly live-API run (.github/workflows/live-api.yml),
# from the exit codes of its two checks, and its tracking issues kept in step.
#
#   ci/live-api-report.sh API_CHECK_CODE LIVE_SUITE_CODE
#
# A code is empty when its step never ran (the build failed, say).
#   api_check:  0 ok, 1 broken, 2 could not run, 3 drift (schema/ is out of
#               date, nothing broken)
#   live_suite: 0 ok, 1 broken, 2 refused or a setup error, 3 inconclusive
#               (only network failures, timeouts, 5xx or rate limiting),
#               4 the test account's token was rejected
# Any other code (a panic is 101) counts as a failure.
#
# Exits 0 when both passed, or api_check found only drift; 1 otherwise; 2 on
# bad arguments.
#
# Issues, labelled live-api and assigned to the owner:
# - "Live API check failed": opened by a failing run, commented on by each
#   later one, closed by the next run that passes ("passed again"). Its words
#   say whether the app is broken, the token was rejected, or the run was
#   inconclusive.
# - "Live API: the schema changed": opened or commented on while api_check
#   reports drift, closed once schema/ matches the live API again.
#
# Needs gh with GH_TOKEN and GH_REPO, and GITHUB_SERVER_URL,
# GITHUB_REPOSITORY and GITHUB_RUN_ID for the run's link. Appends the verdict
# to GITHUB_STEP_SUMMARY when it is set. ci/live-api-report-test.sh replaces
# gh (GH).
set -euo pipefail

if (($# != 2)); then
  echo "usage: $0 API_CHECK_CODE LIVE_SUITE_CODE (either may be empty)" >&2
  exit 2
fi
api_check=$1
live_suite=$2

gh=${GH:-gh}
run_url="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID"
label=live-api
assignee=AliSayyah
failed_title="Live API check failed"
drift_title="Live API: the schema changed"

# What a code means, as a class: ok, drift, broken, token, setup,
# inconclusive, missing (the step never ran) or unexpected.
api_check_class() {
  case $1 in
    0) echo ok ;;
    1) echo broken ;;
    2) echo setup ;;
    3) echo drift ;;
    '') echo missing ;;
    *) echo unexpected ;;
  esac
}
live_suite_class() {
  case $1 in
    0) echo ok ;;
    1) echo broken ;;
    2) echo setup ;;
    3) echo inconclusive ;;
    4) echo token ;;
    '') echo missing ;;
    *) echo unexpected ;;
  esac
}

# One line on what a check's class means.
describe() {
  local check=$1 class=$2 code=$3 what
  case $check:$class in
    *:ok) what="passed" ;;
    api_check:broken) what="**broken**: a GraphQL document no longer validates against the live schema, or the app's scopes no longer cover it" ;;
    api_check:setup) what="could not run (no token, a network error, or an answer it couldn't read)" ;;
    api_check:drift) what="nothing broken, but the live schema or scope map differs from \`schema/\`" ;;
    live_suite:broken) what="**broken**: a check failed (a response that doesn't decode, an API error, or an assertion)" ;;
    live_suite:setup) what="refused to run, or could not start: check the \`HARDCOVER_TEST_ACCOUNT\` variable and the \`hardcover-live\` environment" ;;
    live_suite:inconclusive) what="inconclusive: only network failures, timeouts, server errors or rate limiting" ;;
    live_suite:token) what="the test account's token was rejected (expired or revoked)" ;;
    *:missing) what="did not run: the job stopped before it (see the build steps)" ;;
    *) what="failed unexpectedly" ;;
  esac
  if [[ $class == missing ]]; then
    echo "- \`$check\`: $what"
  else
    echo "- \`$check\`: $what (exit $code)"
  fi
}

a_class=$(api_check_class "$api_check")
l_class=$(live_suite_class "$live_suite")
classes=" $a_class $l_class "

# The verdict, the worst first: broken, then a rejected token, then anything
# unexpected; the rest (setup, inconclusive, missing) says nothing about the
# API.
if [[ $l_class == ok && ($a_class == ok || $a_class == drift) ]]; then
  verdict=pass
  headline="The live API checks passed."
  if [[ $a_class == drift ]]; then
    headline+=" The live schema or scope map differs from \`schema/\`: run \`just schema-update\`."
  fi
elif [[ $classes == *" broken "* ]]; then
  verdict=broken
  headline="The app is broken against the live Hardcover API."
elif [[ $classes == *" token "* ]]; then
  verdict=token
  headline="The test account's token was rejected: replace the \`HARDCOVER_TEST_TOKEN\` secret (environment \`hardcover-live\`). Until then the checks say nothing about the API."
elif [[ $classes == *" unexpected "* ]]; then
  verdict=unexpected
  headline="A live API check failed unexpectedly: see the run's log."
else
  verdict=inconclusive
  headline="Inconclusive: the checks could not run to the end (setup, network or rate limiting), so they say nothing about the API. Look at the log if it happens again."
fi

details=$(
  describe api_check "$a_class" "$api_check"
  describe live_suite "$l_class" "$live_suite"
)
echo "$headline"
echo "$details"

if [[ -n ${GITHUB_STEP_SUMMARY:-} ]]; then
  printf '## Live API: verdict\n\n%s\n\n%s\n' "$headline" "$details" >>"$GITHUB_STEP_SUMMARY"
fi

# The number of the open issue titled exactly $1, if any. The search is fuzzy,
# so the title is compared here.
open_issue() {
  local title=$1 rows number found
  rows=$("$gh" issue list --label "$label" --state open --search "in:title \"$title\"" \
    --limit 100 --json number,title \
    --template '{{range .}}{{.number}}{{"\t"}}{{.title}}{{"\n"}}{{end}}')
  while IFS=$'\t' read -r number found; do
    if [[ $found == "$title" ]]; then
      echo "$number"
      return
    fi
  done <<<"$rows"
}

# Comments $2 on the open issue titled $1, or opens it with $2 as its body.
report_issue() {
  local title=$1 body=$2 number
  number=$(open_issue "$title")
  if [[ -n $number ]]; then
    "$gh" issue comment "$number" --body "$body"
  else
    "$gh" label create "$label" --force --color D93F0B \
      --description "Found by the weekly live API check"
    "$gh" issue create --title "$title" --body "$body" --label "$label" --assignee "$assignee"
  fi
}

# Closes the open issue titled $1, if any, saying $2.
close_issue() {
  local title=$1 comment=$2 number
  number=$(open_issue "$title")
  if [[ -n $number ]]; then
    "$gh" issue close "$number" --comment "$comment"
  fi
}

if [[ $verdict == pass ]]; then
  close_issue "$failed_title" "Passed again: $run_url"
else
  report_issue "$failed_title" "$headline

$details

Run: $run_url

This issue gets a comment from every failing weekly run and is closed by the next one that passes."
fi

case $a_class in
  drift)
    report_issue "$drift_title" "The live Hardcover API's schema or scope map differs from \`schema/\`, though nothing the app uses is broken.

Run \`just schema-update\`, review \`git diff schema/\`, and commit it. The run's summary lists the changes.

Run: $run_url"
    ;;
  ok)
    close_issue "$drift_title" "\`schema/\` matches the live API again: $run_url"
    ;;
esac

if [[ $verdict != pass ]]; then
  exit 1
fi
