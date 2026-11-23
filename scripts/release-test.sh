#!/usr/bin/env bash
# `check` evaluates its condition, quoted so it expands then.
# shellcheck disable=SC2016,SC2034

# Tests release.sh's network handling offline: retries, writes that land
# though their answer is lost, CI waits that lose contact, and functions whose
# output is a value. gh is a function here that plays a script of answers.
#
#   scripts/release-test.sh      (`just release-test`)
set -uo pipefail

# shellcheck source=scripts/release.sh
source "$(dirname "${BASH_SOURCE[0]}")/release.sh"
set +e
retry_delays=(0 0 0)
poll=1
patience=3
sleep() { :; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
calls=$tmp/calls
script=$tmp/script
err=$tmp/err
# Each gh call takes the next line of $script: "ok STDOUT" or "err STDERR".
gh() {
  echo "gh $*" >>"$calls"
  local line
  line=$(head -n1 "$script")
  sed -i 1d "$script"
  case $line in
    ok\ *) echo "${line#ok }" ;;
    err\ *)
      echo "${line#err }" >&2
      return 1
      ;;
    *)
      echo "unscripted gh call: $*" >&2
      return 1
      ;;
  esac
}
plan() {
  : >"$calls"
  printf '%s\n' "$@" >"$script"
}
# The number of gh calls since `plan`.
n() { grep -c "^gh " "$calls"; }
eof='Get "https://api.github.com/repos/x/y/actions/runs/1?exclude_pull_requests=true": unexpected EOF'
pass=0 fail=0
# check NAME CONDITION: CONDITION, evaluated, must hold.
check() {
  if eval "$2"; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "FAIL: $1 ($2)"
  fi
}

# What counts as the network's failure, and what as an answer.
for e in "$eof" \
  'Post "https://api.github.com/graphql": dial tcp: lookup api.github.com: no such host' \
  "fatal: unable to access 'https://github.com/x/y/': Failed to connect to github.com port 443" \
  'HTTP 502: Bad Gateway (https://api.github.com/graphql)' \
  'GraphQL: Something went wrong while executing your query.' \
  'fatal: the remote end hung up unexpectedly'; do
  check "transient: ${e:0:40}" 'transient "$e"'
done
for e in 'gh: Not Found (HTTP 404)' 'release not found' 'HTTP 403: Resource not accessible by integration' \
  'GraphQL: Pull request is not mergeable' "! [rejected] v1.0.0 -> v1.0.0 (already exists)"; do
  check "an answer: ${e:0:40}" '! transient "$e"'
done

plan "err $eof" "err $eof" "ok 42"
out=$(retry gh run view 2>/dev/null); st=$?
check "retry recovers from EOF" '[[ $out == 42 && $st == 0 && $(n) == 3 ]]'

plan "err gh: Not Found (HTTP 404)"
out=$(retry gh run view 2>"$err"); st=$?
check "a 404 is not retried" '[[ -z $out && $st == 2 && $(n) == 1 ]] && grep -q "HTTP 404" "$err"'

plan "err release not found"
retry -a 'release not found' gh release view v9 2>"$err"; st=$?
check "an answer returns 1 quietly" '[[ $st == 1 && ! -s "$err" && $(n) == 1 ]]'

plan "err $eof" "err $eof" "err $eof" "err $eof"
out=$(retry gh run view 2>"$err"); st=$?
check "a network that stays down gives 75" '[[ $st == 75 && $(n) == 4 ]] && grep -q "run \`just release\`" "$err"'

plan 'err Post "https://api.github.com/graphql": dial tcp: connection refused' "ok MERGED"
write pr_merged 7 -- gh pr merge 7 2>/dev/null; st=$?
check "a write that landed is not repeated" '[[ $st == 0 && $(n) == 2 ]] && grep -q "pr view 7" "$calls" && [[ $(grep -c "pr merge" "$calls") == 1 ]]'

plan "err GraphQL: Pull request is not mergeable" "ok OPEN"
write pr_merged 7 -- gh pr merge 7 2>"$err"; st=$?
check "a refused write is not retried" '[[ $st == 1 && $(grep -c "pr merge" "$calls") == 1 ]] && grep -q "not mergeable" "$err"'

plan "err $eof" "ok OPEN" "ok merged-output"
out=$(write pr_merged 7 -- gh pr merge 7 2>/dev/null); st=$?
check "a write that failed on the network is tried again, its output off stdout" '[[ $st == 0 && -z $out && $(grep -c "pr merge" "$calls") == 2 ]]'

plan "ok 1234"
out=$(find_run ci.yml abc push 2>/dev/null)
check "find_run prints just the id" '[[ $out == 1234 ]]'

plan "err $eof"
patience=5
(find_run ci.yml abc push) 2>"$err"; st=$?
patience=3
check "find_run losing contact says so" '[[ $st == 1 ]] && grep -q "can.t reach GitHub" "$err" && ! grep -q "no ci.yml run" "$err"'

# wait_for_run: find_run, the url, then polls.
plan "ok 99" "ok https://x/runs/99" "ok in_progress||Build: cargo build" "err $eof" "ok in_progress||Build: cargo build" "ok completed|success|"
out=$(wait_for_run release.yml abcdef1 push 2>"$err"); st=$?
check "a blip during the wait passes" '[[ $st == 0 && -z $out ]] && grep -q "reached GitHub again" "$err"'

plan "ok 99" "ok https://x/runs/99" "ok completed|failure|"
(wait_for_run release.yml abcdef1 push) 2>"$err"; st=$?
check "a failed run says it failed" '[[ $st == 1 ]] && grep -q "ended in failure" "$err" && grep -q "rerun" "$err"'

plan "ok 99" "ok https://x/runs/99" "err $eof" "err $eof" "err $eof" "err $eof"
(wait_for_run release.yml abcdef1 push) 2>"$err"; st=$?
check "lost contact is not a failure" '[[ $st == 1 ]] && ! grep -q "ended in\|rerun" "$err" && grep -q "still running" "$err" && grep -q "can.t reach GitHub. Once" "$err"'

plan "ok v1"
released v1; st=$?
check "released: yes" '[[ $st == 0 ]]'
plan "err release not found"
released v1; st=$?
check "released: no" '[[ $st == 1 ]]'
plan "err $eof" "err $eof" "err $eof" "err $eof"
(released v1) 2>/dev/null; st=$?
check "released: can't tell stops" '[[ $st == 1 && $(n) == 4 ]]'
# In a condition, can't-tell still stops the script, not reads as "no".
plan "err $eof" "err $eof" "err $eof" "err $eof"
( if ! released v1; then echo "read as no"; fi ) >"$tmp/out" 2>/dev/null
check "released: can't tell is not no" '[[ ! -s "$tmp/out" ]]'

plan "ok 55"
out=$(open_pr 9.9.9 2>/dev/null)
check "open_pr prints just the number" '[[ $out == 55 ]]'

plan "ok 99" "ok https://x/runs/99" "err HTTP 401: Bad credentials (https://api.github.com/graphql)"
(wait_for_run release.yml abcdef1 push) 2>"$err"; st=$?
check "an error that isn't the network's ends the wait at once" '[[ $st == 1 && $(n) == 3 ]] && grep -q "Bad credentials" "$err" && ! grep -q "can.t reach" "$err"'

out=$(printf 'a\trefs/heads/release-0.5.0\na\trefs/heads/release-network\na\trefs/heads/release-1.2.30\n' | release_branches)
check "only release-X.Y.Z is a release branch" '[[ $out == $'"'"'0.5.0\n1.2.30'"'"' ]]'

# The resume paths, against a scratch repository and a bare remote.
git init -q --bare "$tmp/remote.git"
git init -q -b "$base" "$tmp/repo"
cd "$tmp/repo" || exit 1
git config user.name test
git config user.email test@example.invalid
for f in "$app_toml" Cargo.lock "$metainfo"; do
  mkdir -p "$(dirname "$f")"
  git -C "$root" show "HEAD:$f" >"$f"
done
git add -A
git commit -qm start
git remote add "$remote" "$tmp/remote.git"
git push -q "$remote" "$base"
current=$(cargo_version <"$app_toml")
v=$(next_version "$current" major)
bump_commit() {
  bump_manifest "$v" <"$app_toml" >x && mv x "$app_toml"
  bump_lock "$v" <Cargo.lock >x && mv x Cargo.lock
  bump_metainfo "$v" 2026-11-22 <"$metainfo" >x && mv x "$metainfo"
  git commit -qam "Release $v"
}

# A run pushed release-X.Y.Z, then lost the PR it was opening.
git switch -qc "release-$v"
bump_commit
pushed=$(git rev-parse HEAD)
git push -q "$remote" "release-$v"
git switch -q "$base"
git branch -qD "release-$v"
check "branch_at sees the pushed branch" 'branch_at "release-$v" "$pushed" && ! branch_at "release-$v" "$(git rev-parse HEAD)"'
fetch
plan "ok " "ok https://x/pull/5" "ok 5"
out=$(open_pr "$v" 2>"$err"); st=$?
check "open_pr opens the PR for a pushed branch" '[[ $st == 0 && $out == 5 ]] && grep -q "pr create.*--head release-$v" "$calls" && grep -q "already pushed" "$err"'

# Master moved since: the branch is not the bump any more.
echo moved >moved
git add moved
git commit -qm moved
git push -q "$remote" "$base"
fetch
plan "ok "
(open_pr "$v") >/dev/null 2>"$err"; st=$?
check "open_pr refuses a pushed branch that isn't the bump on master" '[[ $st == 1 ]] && grep -q "git push $remote --delete release-$v" "$err" && ! grep -q "pr create" "$calls"'
git push -q "$remote" --delete "release-$v"

# The bump merged; a run made the tag and stopped before pushing it.
bump_commit
sha=$(git rev-parse HEAD)
git push -q "$remote" "$base"
fetch
git tag -a "v$v" -m "Hardcover Desktop $v" "$sha"
yes=1
plan "ok completed success"
(tag_release "$sha" "$v") 2>"$err"; st=$?
check "tag_release pushes a local tag a stopped run left" '[[ $st == 0 ]] && grep -q "pushing the local tag" "$err" && tag_at "v$v" "$sha"'
fetch
check "tags count as they are on the remote" 'tag_on_remote "v$v" && [[ $(latest_tag) == "v$v" ]]'

# A local tag of that name on another commit is not pushed.
git push -q "$remote" --delete "v$v"
git tag -d "v$v" >/dev/null
git tag -a "v$v" -m other "$sha^"
fetch
plan "ok completed success"
(tag_release "$sha" "$v") 2>"$err"; st=$?
check "tag_release refuses a local tag on another commit" '[[ $st == 1 ]] && grep -q "git tag -d v$v" "$err" && ! tag_at "v$v" "$sha"'
cd - >/dev/null || exit 1

echo "release-test: $pass passed, $fail failed"
((fail == 0))
